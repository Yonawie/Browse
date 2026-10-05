use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Input tab data for task clustering (AT-1).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TabForClustering {
    pub id: String,
    pub url: String,
    pub title: String,
    pub last_active_at: i64,
}

/// Suggested tab group created by deterministic clustering heuristics (AT-1).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutoTabGroup {
    pub group_id: String,
    pub title: String,
    pub tab_ids: Vec<String>,
    pub dominant_domain: String,
    pub color: String,
    pub reason: String,
    pub confidence: f32,
}

const PALETTE: [&str; 6] = [
    "#3b82f6", // Blue
    "#10b981", // Emerald
    "#8b5cf6", // Purple
    "#f59e0b", // Amber
    "#ec4899", // Pink
    "#06b6d4", // Cyan
];

const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "page", "home", "docs", "documentation",
    "official", "site", "online", "free", "login", "dashboard", "about",
    "http", "https", "www", "com", "org", "net", "html", "htm",
];

/// Extracts clean domain host from a URL.
pub fn extract_domain(url: &str) -> String {
    let lower = url.to_lowercase();
    let without_proto = if let Some(pos) = lower.find("://") {
        &lower[pos + 3..]
    } else {
        &lower
    };
    let host = without_proto.split(&['/', '?', '#', ':'][..]).next().unwrap_or("");
    host.trim_start_matches("www.").to_string()
}

/// Extracts meaningful lowercased topic tokens from title and URL path.
pub fn extract_tokens(title: &str, url: &str) -> HashSet<String> {
    let mut tokens = HashSet::new();

    // From title
    for word in title.split(|c: char| !c.is_alphanumeric()) {
        let clean = word.trim().to_lowercase();
        if clean.len() >= 3 && !STOPWORDS.contains(&clean.as_str()) {
            tokens.insert(clean);
        }
    }

    // From URL path
    if let Some(pos) = url.find("://") {
        let path = &url[pos + 3..];
        if let Some(path_start) = path.find('/') {
            for segment in path[path_start..].split(&['/', '-', '_', '?', '&', '='][..]) {
                let clean = segment.trim().to_lowercase();
                if clean.len() >= 3 && !STOPWORDS.contains(&clean.as_str()) {
                    tokens.insert(clean);
                }
            }
        }
    }

    tokens
}

/// Calculates Jaccard similarity between two token sets.
fn token_jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count() as f32;
    let union = a.union(b).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

/// Calculates domain similarity.
fn domain_similarity(d1: &str, d2: &str) -> f32 {
    if d1.is_empty() || d2.is_empty() {
        return 0.0;
    }
    if d1 == d2 {
        return 1.0;
    }
    // Check shared second-level domain (e.g. docs.rs vs crates.io or subdomains)
    if d1.ends_with(d2) || d2.ends_with(d1) {
        return 0.75;
    }
    0.0
}

/// Calculates temporal proximity between two timestamps (in milliseconds).
fn temporal_proximity(t1: i64, t2: i64) -> f32 {
    let diff = (t1 - t2).abs();
    let minute_ms = 60 * 1000;
    let hour_ms = 60 * minute_ms;

    if diff <= 15 * minute_ms {
        1.0
    } else if diff <= hour_ms {
        0.75
    } else if diff <= 4 * hour_ms {
        0.4
    } else {
        0.1
    }
}

/// Computes pairwise similarity between two tabs.
pub fn tab_similarity(t1: &TabForClustering, t2: &TabForClustering) -> f32 {
    let d1 = extract_domain(&t1.url);
    let d2 = extract_domain(&t2.url);
    let d_sim = domain_similarity(&d1, &d2);

    let tok1 = extract_tokens(&t1.title, &t1.url);
    let tok2 = extract_tokens(&t2.title, &t2.url);
    let tok_sim = token_jaccard(&tok1, &tok2);

    let time_sim = temporal_proximity(t1.last_active_at, t2.last_active_at);

    // Weights: domain 0.50, topic tokens 0.35, time 0.15
    0.50 * d_sim + 0.35 * tok_sim + 0.15 * time_sim
}

/// Automatically cluster tabs into semantic task groups (AT-1).
///
/// Minimum group size is 2 tabs. Tabs that don't cluster remain ungrouped.
pub fn cluster_tabs(tabs: &[TabForClustering], similarity_threshold: f32) -> Vec<AutoTabGroup> {
    if tabs.len() < 2 {
        return Vec::new();
    }

    let n = tabs.len();
    // Build adjacency graph of tabs that exceed threshold
    let mut adj = vec![vec![false; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let sim = tab_similarity(&tabs[i], &tabs[j]);
            if sim >= similarity_threshold {
                adj[i][j] = true;
                adj[j][i] = true;
            }
        }
    }

    // Connected components (Breadth-First Search)
    let mut visited = vec![false; n];
    let mut raw_clusters: Vec<Vec<usize>> = Vec::new();

    for i in 0..n {
        if visited[i] {
            continue;
        }
        let mut component = Vec::new();
        let mut queue = vec![i];
        visited[i] = true;

        while let Some(curr) = queue.pop() {
            component.push(curr);
            for (neighbor, &connected) in adj[curr].iter().enumerate() {
                if connected && !visited[neighbor] {
                    visited[neighbor] = true;
                    queue.push(neighbor);
                }
            }
        }

        // Only keep clusters with at least 2 tabs
        if component.len() >= 2 {
            raw_clusters.push(component);
        }
    }

    let mut groups = Vec::new();

    for (idx, cluster_indices) in raw_clusters.into_iter().enumerate() {
        let cluster_tabs: Vec<&TabForClustering> = cluster_indices.iter().map(|&i| &tabs[i]).collect();
        let tab_ids: Vec<String> = cluster_tabs.iter().map(|t| t.id.clone()).collect();

        // 1. Dominant domain
        let mut domain_counts: HashMap<String, usize> = HashMap::new();
        for t in &cluster_tabs {
            let dom = extract_domain(&t.url);
            *domain_counts.entry(dom).or_default() += 1;
        }
        let (dominant_dom, dom_count) = domain_counts
            .into_iter()
            .max_by_key(|&(_, count)| count)
            .unwrap_or_else(|| ("general".to_string(), 1));

        // 2. Dominant topic keywords
        let mut token_counts: HashMap<String, usize> = HashMap::new();
        for t in &cluster_tabs {
            let tokens = extract_tokens(&t.title, &t.url);
            for tok in tokens {
                *token_counts.entry(tok).or_default() += 1;
            }
        }

        let mut sorted_tokens: Vec<(String, usize)> = token_counts.into_iter().collect();
        sorted_tokens.sort_by(|a, b| b.1.cmp(&a.1));

        // 3. Human-friendly title synthesis
        let title = if let Some((top_token, count)) = sorted_tokens.first() {
            if *count >= 2 {
                // Shared topic across tabs
                let capitalized = {
                    let mut chars = top_token.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
                    }
                };
                if dom_count == cluster_tabs.len() && !dominant_dom.is_empty() {
                    let brand = clean_brand_name(&dominant_dom);
                    format!("{brand}: {capitalized}")
                } else {
                    format!("{capitalized} Research")
                }
            } else if !dominant_dom.is_empty() {
                let brand = clean_brand_name(&dominant_dom);
                format!("{brand} ({})", cluster_tabs.len())
            } else {
                format!("Task Group {}", idx + 1)
            }
        } else if !dominant_dom.is_empty() {
            let brand = clean_brand_name(&dominant_dom);
            format!("{brand} ({})", cluster_tabs.len())
        } else {
            format!("Task Group {}", idx + 1)
        };

        let color = PALETTE[idx % PALETTE.len()].to_string();
        let reason = if dom_count == cluster_tabs.len() {
            format!("All {} tabs share the domain '{dominant_dom}'", cluster_tabs.len())
        } else {
            format!("Tabs share related topics and activity time window on '{dominant_dom}'")
        };

        let confidence = (dom_count as f32 / cluster_tabs.len() as f32).max(0.65);

        groups.push(AutoTabGroup {
            group_id: format!("auto-grp-{}", idx + 1),
            title,
            tab_ids,
            dominant_domain: dominant_dom,
            color,
            reason,
            confidence,
        });
    }

    groups
}

fn clean_brand_name(domain: &str) -> String {
    let parts: Vec<&str> = domain.split('.').collect();
    let name = if parts.len() >= 2 {
        parts[parts.len() - 2]
    } else {
        domain
    };

    let mut chars = name.chars();
    match chars.next() {
        None => domain.to_string(),
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

impl crate::MemoryStore {
    /// Perform tab auto-grouping and return candidate groups (AT-1).
    pub fn suggest_tab_groups(
        &self,
        tabs: &[TabForClustering],
    ) -> Result<Vec<AutoTabGroup>> {
        // Default similarity threshold 0.35 allows related domain + topic clusters
        Ok(cluster_tabs(tabs, 0.35))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clusters_tabs_by_domain_and_topic() {
        let tabs = vec![
            TabForClustering {
                id: "t1".into(),
                url: "https://github.com/rust-lang/rust/pull/123".into(),
                title: "Rust compiler PR #123".into(),
                last_active_at: 10_000,
            },
            TabForClustering {
                id: "t2".into(),
                url: "https://github.com/rust-lang/cargo/issues/456".into(),
                title: "Cargo build issue #456".into(),
                last_active_at: 12_000,
            },
            TabForClustering {
                id: "t3".into(),
                url: "https://news.ycombinator.com/item?id=999".into(),
                title: "Hacker News discussion".into(),
                last_active_at: 800_000,
            },
            TabForClustering {
                id: "t4".into(),
                url: "https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html".into(),
                title: "Understanding Ownership - Rust Book".into(),
                last_active_at: 15_000,
            },
        ];

        let groups = cluster_tabs(&tabs, 0.35);
        assert!(!groups.is_empty());
        // GitHub tabs should cluster together
        let gh_group = groups.iter().find(|g| g.tab_ids.contains(&"t1".to_string())).unwrap();
        assert!(gh_group.tab_ids.contains(&"t2".to_string()));
        assert!(!gh_group.tab_ids.contains(&"t3".to_string()));
        assert!(gh_group.dominant_domain.contains("github"));
    }

    #[test]
    fn unclustered_isolated_tabs_produce_no_group() {
        let tabs = vec![
            TabForClustering {
                id: "t1".into(),
                url: "https://example.com/a".into(),
                title: "Cooking Recipes".into(),
                last_active_at: 10_000,
            },
            TabForClustering {
                id: "t2".into(),
                url: "https://different-site.org/b".into(),
                title: "Space Exploration".into(),
                last_active_at: 50_000_000,
            },
        ];

        let groups = cluster_tabs(&tabs, 0.40);
        assert!(groups.is_empty());
    }
}
