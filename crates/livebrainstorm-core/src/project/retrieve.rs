//! Per-keystroke retrieval: which files is the draft talking about?
//!
//! TF-IDF over identifier terms. No embeddings on purpose: it runs in
//! microseconds, needs no model, works offline, and code identifiers are
//! exactly the vocabulary engineers type. Swap in embeddings later behind
//! the same function if fuzzy matching ("payments" vs `billing/`) matters.

use std::collections::HashMap;

use serde::Serialize;

use super::{terms, ProjectIndex};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub path: String,
    /// 1-based.
    pub start_line: usize,
    pub text: String,
}

const WINDOW_BEFORE: usize = 8;
const WINDOW_AFTER: usize = 16;
/// The end of the draft is where the user is typing *now*; weight it up.
const RECENT_CHARS: usize = 400;
const RECENT_BOOST: f64 = 1.6;

pub fn retrieve(index: &ProjectIndex, query: &str, max_files: usize, budget_chars: usize) -> Vec<Snippet> {
    let weights = query_weights(query);
    if weights.is_empty() || index.files.is_empty() || max_files == 0 {
        return vec![];
    }
    let n = index.files.len() as f64;
    let idf = |t: &str| (1.0 + n / f64::from(*index.doc_freq.get(t).unwrap_or(&0)).max(1.0)).ln();

    let mut scored: Vec<(f64, usize)> = index
        .files
        .iter()
        .enumerate()
        .filter_map(|(i, f)| {
            let path_lower = f.rel.to_lowercase();
            let score: f64 = weights
                .iter()
                .map(|(t, w)| {
                    let tf = f.terms.get(t).copied().unwrap_or(0);
                    let body = if tf > 0 { 1.0 + f64::from(tf).ln() } else { 0.0 };
                    let path = if path_lower.contains(t.as_str()) { 2.0 } else { 0.0 };
                    w * idf(t) * (body + path)
                })
                .sum();
            (score > 0.0).then_some((score, i))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));

    let per_file = budget_chars / max_files;
    scored
        .into_iter()
        .take(max_files)
        .map(|(_, i)| snippet(&index.files[i].rel, &index.files[i].content, &weights, per_file))
        .collect()
}

fn query_weights(query: &str) -> HashMap<String, f64> {
    let split = query.len().saturating_sub(RECENT_CHARS);
    let split = (split..=query.len()).find(|i| query.is_char_boundary(*i)).unwrap_or(query.len());
    let (older, recent) = query.split_at(split);
    let mut w: HashMap<String, f64> = HashMap::new();
    for t in terms::tokens(older) {
        w.entry(t).or_insert(1.0);
    }
    for t in terms::tokens(recent) {
        w.insert(t, RECENT_BOOST);
    }
    w
}

/// The window around the line with the most query hits.
fn snippet(path: &str, content: &str, weights: &HashMap<String, f64>, max_chars: usize) -> Snippet {
    let lines: Vec<&str> = content.lines().collect();
    let best = lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let lower = l.to_lowercase();
            let hits: f64 = weights.iter().filter(|(t, _)| lower.contains(t.as_str())).map(|(_, w)| w).sum();
            (hits, i)
        })
        .fold((0.0, 0), |acc, x| if x.0 > acc.0 { x } else { acc })
        .1;
    let start = best.saturating_sub(WINDOW_BEFORE);
    let end = (best + WINDOW_AFTER).min(lines.len());
    let mut text = lines[start..end].join("\n");
    if text.len() > max_chars {
        let cut = (0..=max_chars).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
        text.truncate(cut);
    }
    Snippet { path: path.to_owned(), start_line: start + 1, text }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::scan::{scan, tests::fixture};

    #[test]
    fn finds_relevant_file_and_line() {
        let dir = fixture();
        let idx = scan(dir.path()).unwrap();
        let hits = retrieve(&idx, "I want invoices to include discounts per cart", 2, 4_000);
        assert_eq!(hits[0].path, "src/billing/invoice.ts");
        assert!(hits[0].text.contains("createInvoice"));
    }

    #[test]
    fn empty_query_returns_nothing() {
        let dir = fixture();
        let idx = scan(dir.path()).unwrap();
        assert!(retrieve(&idx, "the and to", 3, 1_000).is_empty());
    }
}
