//! Desktop CLI commands for Memory (S5): indexing, semantic search, stats and privacy erasure.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use core_types::Sensitivity;
use engine_adapter::{EngineAdapter, WebViewOptions};
use engine_cdp::launcher::LaunchOptions;
use engine_cdp::CdpEngine;
use memory::{MemoryStore, NewChunk, NewPageVersion, SearchFilters, SearchHit};
use page_intelligence::{approx_tokens, content_hash, trim_observation, ObservationBudget};

use crate::models;

pub fn database_path() -> PathBuf {
    if let Ok(val) = std::env::var("BROWSE_DATABASE_PATH") {
        if !val.trim().is_empty() {
            return PathBuf::from(val);
        }
    }
    // Default to a local sqlite database file in the workspace or home
    PathBuf::from("browse-memory.sqlite")
}

pub async fn memory_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let sub = args.first().map(String::as_str);
    match sub {
        Some("stats") => stats_subcommand().await,
        Some("index") => {
            let url = args.get(1).ok_or("usage: browse-desktop memory index <url>")?;
            index_subcommand(url).await
        }
        Some("search") => {
            if args.len() < 2 {
                return Err("usage: browse-desktop memory search <query> [--domain <domain>] [--limit <n>]".into());
            }
            let (query, domain, limit) = parse_search_args(&args[1..])?;
            search_subcommand(&query, domain.as_deref(), limit).await
        }
        Some("forget") => {
            let target = args.get(1).ok_or("usage: browse-desktop memory forget <domain | url>")?;
            forget_subcommand(target).await
        }
        _ => {
            eprintln!("usage: browse-desktop memory <stats | index <url> | search <query> [--domain <d>] | forget <domain>>");
            Ok(())
        }
    }
}

async fn stats_subcommand() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = database_path();
    let exists = db_path.exists();
    let store = MemoryStore::open(&db_path)?;

    println!("Memory Database: {}", db_path.display());
    println!("Status: {}", if exists { "ready" } else { "created new" });
    let tables = [
        "profiles",
        "pages",
        "page_versions",
        "chunks",
        "chunk_vectors_raw",
        "memories",
        "agent_sessions",
        "agent_actions",
        "policy_decisions",
    ];

    println!("\nStored items:");
    for t in tables {
        let count = store.count(t).unwrap_or(0);
        println!("  {t:<20} {count}");
    }
    Ok(())
}

async fn index_subcommand(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let db_path = database_path();
    let store = Arc::new(Mutex::new(MemoryStore::open(&db_path)?));
    store.lock().unwrap().ensure_profile("user", "user", "Default")?;

    eprintln!("connecting to engine to observe {url}...");
    let engine = CdpEngine::launch(LaunchOptions::headless()).await?;
    let webview = engine.create_webview(WebViewOptions::agent("memory-index")).await?;

    engine.navigate(&webview, url).await?;
    let mut observation = engine.observe(&webview).await?;
    trim_observation(&mut observation, ObservationBudget::LOCAL);

    let title = observation.page.title.as_str();
    let full_text: String = observation.content.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\n\n");
    let hash = content_hash(&full_text);

    eprintln!(
        "observed page \"{title}\" ({} chunk(s), ~{} tokens)",
        observation.content.len(),
        observation.approx_tokens
    );

    // Try embedding with gateway if configured
    let configured = models::from_env(Some(store.clone())).await;
    let chunk_texts: Vec<String> = observation.content.iter().map(|c| c.text.clone()).collect();

    let embeddings: Option<Vec<Vec<f32>>> = if !chunk_texts.is_empty() {
        match configured.gateway.embed("memory_index", &chunk_texts).await {
            Ok(v) => {
                eprintln!("generated {} vector embedding(s) via local embedder", v.len());
                Some(v)
            }
            Err(e) => {
                eprintln!("vector embedder unavailable ({e}); indexing with lexical search only");
                None
            }
        }
    } else {
        None
    };

    let chunks_to_insert: Vec<NewChunk> = observation
        .content
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let emb = embeddings.as_ref().and_then(|embs| embs.get(i).map(|v| v.as_slice()));
            NewChunk {
                ordinal: i as u32,
                heading_path: c.heading_path.as_deref(),
                text: &c.text,
                char_start: c.char_start,
                char_end: c.char_end,
                token_count: approx_tokens(&c.text),
                suspect_injection: c.suspect_injection,
                embedding: emb,
            }
        })
        .collect();

    let version_id = {
        let lock = store.lock().unwrap();
        let version_id = lock.insert_page_version(&NewPageVersion {
            url: &observation.page.url,
            title: Some(title),
            lang: None,
            page_kind: observation.page.page_kind,
            sensitivity: observation.page.sensitivity,
            main_text: Some(&full_text),
            content_hash: &hash,
        })?;
        lock.insert_chunks(&version_id, &chunks_to_insert)?;
        version_id
    };

    engine.close_webview(&webview).await?;
    for sidecar in configured.sidecars {
        sidecar.stop().await;
    }

    println!(
        "Successfully indexed \"{}\" into {}\n  version: {}\n  chunks: {}",
        url,
        db_path.display(),
        version_id,
        observation.content.len()
    );
    Ok(())
}

async fn search_subcommand(query: &str, domain: Option<&str>, limit: usize) -> Result<(), Box<dyn std::error::Error>> {
    let db_path = database_path();
    let store = Arc::new(Mutex::new(MemoryStore::open(&db_path)?));

    let configured = models::from_env(Some(store.clone())).await;
    let query_embedding = match configured.gateway.embed("memory_search", &[query.to_string()]).await {
        Ok(mut v) => v.pop(),
        Err(_) => None,
    };

    let filters = SearchFilters {
        domain: domain.map(str::to_string),
        since_ms: None,
        until_ms: None,
        max_sensitivity: Some(Sensitivity::Personal),
    };

    let hits: Vec<SearchHit> = {
        let lock = store.lock().unwrap();
        lock.hybrid_search(query, query_embedding.as_deref(), &filters, limit)?
    };

    for sidecar in configured.sidecars {
        sidecar.stop().await;
    }

    if hits.is_empty() {
        println!("No matching pages found for \"{query}\".");
        return Ok(());
    }

    println!(
        "Found {} match(es) for \"{}\"{}:",
        hits.len(),
        query,
        if query_embedding.is_some() { " (hybrid lexical + vector)" } else { " (lexical only)" }
    );

    for (idx, hit) in hits.iter().enumerate() {
        let title = hit.title.as_deref().unwrap_or("Untitled");
        let heading = hit.heading_path.as_deref().unwrap_or("Page");
        let snippet = collapse_snippet(&hit.text, 160);
        println!(
            "\n  [{}] \"{}\" (score: {:.3})\n      URL: {}\n      Section: {}\n      Snippet: \"{}\"",
            idx + 1,
            title,
            hit.score,
            hit.url,
            heading,
            snippet
        );
    }
    Ok(())
}

async fn forget_subcommand(target: &str) -> Result<(), Box<dyn std::error::Error>> {
    let db_path = database_path();
    let store = MemoryStore::open(&db_path)?;

    if target.starts_with("http://") || target.starts_with("https://") {
        store.forget_url(target)?;
        println!("Deleted all history, chunks and vectors for page: {target}");
    } else {
        store.forget_domain(target)?;
        println!("Deleted all history, chunks and vectors for domain: {target}");
    }
    Ok(())
}

fn collapse_snippet(text: &str, max_len: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_len {
        collapsed
    } else {
        let truncated: String = collapsed.chars().take(max_len).collect();
        format!("{truncated}...")
    }
}

fn parse_search_args(args: &[String]) -> Result<(String, Option<String>, usize), Box<dyn std::error::Error>> {
    let mut query_parts = Vec::new();
    let mut domain = None;
    let mut limit = 5;

    let mut i = 0;
    while i < args.len() {
        if args[i] == "--domain" && i + 1 < args.len() {
            domain = Some(args[i + 1].clone());
            i += 2;
        } else if args[i] == "--limit" && i + 1 < args.len() {
            limit = args[i + 1].parse().unwrap_or(5);
            i += 2;
        } else {
            query_parts.push(args[i].clone());
            i += 1;
        }
    }

    if query_parts.is_empty() {
        return Err("search query cannot be empty".into());
    }

    Ok((query_parts.join(" "), domain, limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_arguments() {
        let args = vec![
            "rust".into(),
            "browser".into(),
            "--domain".into(),
            "example.com".into(),
            "--limit".into(),
            "10".into(),
        ];
        let (query, domain, limit) = parse_search_args(&args).unwrap();
        assert_eq!(query, "rust browser");
        assert_eq!(domain.as_deref(), Some("example.com"));
        assert_eq!(limit, 10);
    }

    #[test]
    fn collapses_and_truncates_snippets() {
        let text = "Hello    world \n this is a long text to test snippet formatting.";
        let short = collapse_snippet(text, 20);
        assert_eq!(short, "Hello world this is ...");
    }
}

