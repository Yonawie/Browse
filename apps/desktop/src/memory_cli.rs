//! Explicit, local page memory commands backed by the existing SQLite store.

use std::path::Path;

use core_types::{Observation, PageKind, Sensitivity};
use engine_adapter::{EngineAdapter, WebViewOptions};
use engine_cdp::launcher::LaunchOptions;
use engine_cdp::CdpEngine;
use memory::{MemoryStore, NewChunk, NewPageVersion, SearchFilters};

pub async fn memory_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let usage = "usage: browse-desktop memory <db-path> <save <url> | search <query> | forget <url>>";
    if args.len() < 3 {
        return Err(usage.into());
    }
    let db_path = Path::new(&args[0]);
    let action = args[1].as_str();
    let value = args[2..].join(" ");
    if value.trim().is_empty() || (action != "search" && args.len() != 3) {
        return Err(usage.into());
    }
    if matches!(action, "save" | "forget") && !(value.starts_with("http://") || value.starts_with("https://")) {
        return Err(usage.into());
    }

    match action {
        "save" => save_page(db_path, &value).await?,
        "search" => search_pages(db_path, &value)?,
        "forget" => forget_page(db_path, &value)?,
        _ => return Err(usage.into()),
    }
    Ok(())
}

async fn save_page(path: &Path, url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let engine = CdpEngine::launch(LaunchOptions::headless()).await?;
    let webview = engine.create_webview(WebViewOptions::agent("memory-save")).await?;
    let observed = async {
        engine.navigate(&webview, url).await?;
        Ok::<_, Box<dyn std::error::Error>>(engine.observe(&webview).await?)
    }
    .await;
    let close_result = engine.close_webview(&webview).await;
    let observation = observed?;
    close_result?;
    validate_observation(&observation)?;
    let store = MemoryStore::open(path)?;
    let saved = save_observation(&store, &observation)?;
    println!("saved {saved} chunks from {}", observation.page.url);
    Ok(())
}

fn save_observation(store: &MemoryStore, observation: &Observation) -> Result<usize, Box<dyn std::error::Error>> {
    validate_observation(observation)?;
    let page = &observation.page;
    let chunks: Vec<_> =
        observation.content.iter().filter(|chunk| !chunk.suspect_injection && !chunk.text.trim().is_empty()).collect();
    let text = chunks.iter().map(|chunk| chunk.text.as_str()).collect::<Vec<_>>().join("\n\n");
    let hash = page_intelligence::content_hash(&text);
    let version = store.insert_page_version(&NewPageVersion {
        url: &page.url,
        title: Some(&page.title),
        lang: page.lang.as_deref(),
        page_kind: page.page_kind,
        sensitivity: page.sensitivity,
        main_text: Some(&text),
        content_hash: &hash,
    })?;
    let existing: i64 = store.conn().query_row(
        "SELECT COUNT(*) FROM chunks WHERE page_version_id = ?1",
        [version.as_str()],
        |row| row.get(0),
    )?;
    if existing > 0 {
        return Ok(existing as usize);
    }
    let records: Vec<_> = chunks
        .iter()
        .enumerate()
        .map(|(ordinal, chunk)| NewChunk {
            ordinal: ordinal as u32,
            heading_path: chunk.heading_path.as_deref(),
            text: &chunk.text,
            char_start: chunk.char_start,
            char_end: chunk.char_end,
            token_count: page_intelligence::approx_tokens(&chunk.text),
            suspect_injection: false,
            embedding: None,
        })
        .collect();
    store.insert_chunks(&version, &records)?;
    Ok(records.len())
}

fn validate_observation(observation: &Observation) -> Result<(), &'static str> {
    let page = &observation.page;
    if page.sensitivity != Sensitivity::Public
        || matches!(page.page_kind, PageKind::Auth | PageKind::Checkout)
        || observation.interactive.iter().any(|element| element.state.masked)
    {
        return Err("only public pages without masked fields can be saved");
    }
    if !observation.content.iter().any(|chunk| !chunk.suspect_injection && !chunk.text.trim().is_empty()) {
        return Err("the page did not expose readable content safe to save");
    }
    Ok(())
}

fn search_pages(path: &Path, query: &str) -> Result<(), Box<dyn std::error::Error>> {
    if !path.is_file() {
        return Err("memory database not found".into());
    }
    let store = MemoryStore::open(path)?;
    let hits = store.hybrid_search(query, None, &SearchFilters::default(), 10)?;
    if hits.is_empty() {
        println!("no results");
    }
    for hit in hits {
        println!(
            "{}\n{}\n{}",
            safe_display(&hit.title.unwrap_or_default()),
            safe_display(&hit.url),
            safe_display(&hit.text.split_whitespace().collect::<Vec<_>>().join(" "))
        );
    }
    Ok(())
}

fn forget_page(path: &Path, url: &str) -> Result<(), Box<dyn std::error::Error>> {
    if !path.is_file() {
        return Err("memory database not found".into());
    }
    let store = MemoryStore::open(path)?;
    let removed = store.conn().execute("DELETE FROM pages WHERE url = ?1", [url])?;
    println!("forgot {removed} page(s)");
    Ok(())
}

fn safe_display(value: &str) -> String {
    value.chars().filter(|character| !character.is_control()).collect()
}
