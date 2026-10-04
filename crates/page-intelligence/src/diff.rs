use crate::content_hash;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffKind {
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffItem {
    pub kind: DiffKind,
    pub section: Option<String>,
    pub old_text: Option<String>,
    pub new_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageDiffReport {
    pub has_changes: bool,
    pub old_content_hash: Option<String>,
    pub new_content_hash: String,
    pub added_count: usize,
    pub removed_count: usize,
    pub modified_count: usize,
    pub items: Vec<DiffItem>,
    pub summary: String,
}

/// Compute differences between an earlier page text snapshot and the current snapshot.
pub fn compute_page_diff(old_text: Option<&str>, new_text: &str) -> PageDiffReport {
    let new_hash = content_hash(new_text);

    let old_text = match old_text {
        Some(t) => t,
        None => {
            let blocks = split_blocks(new_text);
            let items: Vec<DiffItem> = blocks
                .into_iter()
                .map(|b| DiffItem {
                    kind: DiffKind::Added,
                    section: None,
                    old_text: None,
                    new_text: Some(b.to_string()),
                })
                .collect();
            let added_count = items.len();
            return PageDiffReport {
                has_changes: true,
                old_content_hash: None,
                new_content_hash: new_hash,
                added_count,
                removed_count: 0,
                modified_count: 0,
                summary: format!("New page snapshot: {} section(s) captured.", added_count),
                items,
            };
        }
    };

    let old_hash = content_hash(old_text);
    if old_hash == new_hash {
        return PageDiffReport {
            has_changes: false,
            old_content_hash: Some(old_hash),
            new_content_hash: new_hash,
            added_count: 0,
            removed_count: 0,
            modified_count: 0,
            items: Vec::new(),
            summary: "No changes detected since last visit.".to_string(),
        };
    }

    let old_blocks = split_blocks(old_text);
    let new_blocks = split_blocks(new_text);

    let raw_diff = lcs_diff(&old_blocks, &new_blocks);

    // Group adjacent removals and additions into modifications if they share vocabulary
    let mut items = Vec::new();
    let mut i = 0;
    while i < raw_diff.len() {
        if i + 1 < raw_diff.len() {
            match (&raw_diff[i], &raw_diff[i + 1]) {
                (RawDiff::Removed(r), RawDiff::Added(a)) | (RawDiff::Added(a), RawDiff::Removed(r)) => {
                    if are_similar_blocks(r, a) {
                        items.push(DiffItem {
                            kind: DiffKind::Modified,
                            section: None,
                            old_text: Some((*r).to_string()),
                            new_text: Some((*a).to_string()),
                        });
                        i += 2;
                        continue;
                    }
                }
                _ => {}
            }
        }

        match &raw_diff[i] {
            RawDiff::Added(a) => {
                items.push(DiffItem {
                    kind: DiffKind::Added,
                    section: None,
                    old_text: None,
                    new_text: Some((*a).to_string()),
                });
            }
            RawDiff::Removed(r) => {
                items.push(DiffItem {
                    kind: DiffKind::Removed,
                    section: None,
                    old_text: Some((*r).to_string()),
                    new_text: None,
                });
            }
        }
        i += 1;
    }

    let mut added_count = 0;
    let mut removed_count = 0;
    let mut modified_count = 0;

    for item in &items {
        match item.kind {
            DiffKind::Added => added_count += 1,
            DiffKind::Removed => removed_count += 1,
            DiffKind::Modified => modified_count += 1,
        }
    }

    let has_changes = !items.is_empty();

    let mut summary_lines = Vec::new();
    if has_changes {
        summary_lines.push(format!(
            "Changes detected: {} added, {} removed, {} modified section(s).",
            added_count, removed_count, modified_count
        ));
        for item in items.iter().take(5) {
            match item.kind {
                DiffKind::Modified => {
                    let old_preview = preview_str(item.old_text.as_deref().unwrap_or(""));
                    let new_preview = preview_str(item.new_text.as_deref().unwrap_or(""));
                    summary_lines.push(format!("• Modified: \"{}\" → \"{}\"", old_preview, new_preview));
                }
                DiffKind::Added => {
                    let preview = preview_str(item.new_text.as_deref().unwrap_or(""));
                    summary_lines.push(format!("• Added: \"{}\"", preview));
                }
                DiffKind::Removed => {
                    let preview = preview_str(item.old_text.as_deref().unwrap_or(""));
                    summary_lines.push(format!("• Removed: \"{}\"", preview));
                }
            }
        }
        if items.len() > 5 {
            summary_lines.push(format!("• ...and {} more changes", items.len() - 5));
        }
    } else {
        summary_lines.push("No changes detected since last visit.".to_string());
    }

    PageDiffReport {
        has_changes,
        old_content_hash: Some(old_hash),
        new_content_hash: new_hash,
        added_count,
        removed_count,
        modified_count,
        items,
        summary: summary_lines.join("\n"),
    }
}

fn preview_str(s: &str) -> String {
    if s.chars().count() > 60 {
        let truncated: String = s.chars().take(57).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}

fn split_blocks(text: &str) -> Vec<&str> {
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
enum RawDiff<'a> {
    Added(&'a str),
    Removed(&'a str),
}

fn lcs_diff<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<RawDiff<'a>> {
    let n = old.len();
    let m = new.len();

    // Guard against huge documents: cap at 300x300 for quadratic DP
    if n * m > 90_000 {
        return fast_hash_diff(old, new);
    }

    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in 0..n {
        for j in 0..m {
            if old[i] == new[j] {
                dp[i + 1][j + 1] = dp[i][j] + 1;
            } else {
                dp[i + 1][j + 1] = dp[i + 1][j].max(dp[i][j + 1]);
            }
        }
    }

    let mut diff = Vec::new();
    let mut i = n;
    let mut j = m;

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old[i - 1] == new[j - 1] {
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            diff.push(RawDiff::Added(new[j - 1]));
            j -= 1;
        } else if i > 0 {
            diff.push(RawDiff::Removed(old[i - 1]));
            i -= 1;
        }
    }

    diff.reverse();
    diff
}

fn fast_hash_diff<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<RawDiff<'a>> {
    use std::collections::HashSet;
    let old_set: HashSet<&'a str> = old.iter().copied().collect();
    let new_set: HashSet<&'a str> = new.iter().copied().collect();

    let mut diff = Vec::new();
    for &o in old {
        if !new_set.contains(o) {
            diff.push(RawDiff::Removed(o));
        }
    }
    for &n in new {
        if !old_set.contains(n) {
            diff.push(RawDiff::Added(n));
        }
    }
    diff
}

fn are_similar_blocks(a: &str, b: &str) -> bool {
    let a_words: Vec<&str> = a.split_whitespace().collect();
    let b_words: Vec<&str> = b.split_whitespace().collect();
    if a_words.is_empty() || b_words.is_empty() {
        return false;
    }
    let common = a_words.iter().filter(|w| b_words.contains(w)).count();
    let min_len = a_words.len().min(b_words.len());
    // At least 40% common words or same starting token
    (common as f32 / min_len as f32) >= 0.4 || (a_words[0] == b_words[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_text_produces_no_changes() {
        let text = "Welcome to Browse.\nAn AI-native browser with deterministic security.";
        let report = compute_page_diff(Some(text), text);
        assert!(!report.has_changes);
        assert_eq!(report.added_count, 0);
        assert_eq!(report.removed_count, 0);
        assert_eq!(report.modified_count, 0);
        assert!(report.items.is_empty());
    }

    #[test]
    fn initial_snapshot_marks_all_added() {
        let text = "Title 1\nParagraph A\nParagraph B";
        let report = compute_page_diff(None, text);
        assert!(report.has_changes);
        assert_eq!(report.added_count, 3);
        assert_eq!(report.removed_count, 0);
        assert_eq!(report.modified_count, 0);
    }

    #[test]
    fn detects_modifications_and_additions() {
        let old = "Product X\nPrice: $99 / month\nIn Stock: 5";
        let new = "Product X\nPrice: $79 / month\nIn Stock: 5\nBonus: Free shipping";

        let report = compute_page_diff(Some(old), new);
        assert!(report.has_changes);
        assert_eq!(report.modified_count, 1);
        assert_eq!(report.added_count, 1);
        assert_eq!(report.removed_count, 0);

        let mod_item = report.items.iter().find(|i| i.kind == DiffKind::Modified).unwrap();
        assert_eq!(mod_item.old_text.as_deref(), Some("Price: $99 / month"));
        assert_eq!(mod_item.new_text.as_deref(), Some("Price: $79 / month"));

        let add_item = report.items.iter().find(|i| i.kind == DiffKind::Added).unwrap();
        assert_eq!(add_item.new_text.as_deref(), Some("Bonus: Free shipping"));
    }
}
