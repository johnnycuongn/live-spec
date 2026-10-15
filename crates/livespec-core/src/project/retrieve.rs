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
    if max_files == 0 {
        return vec![];
    }
    shortlist(index, query, max_files, budget_chars / max_files)
}

/// Up to `n` snippets in TF-IDF order, each at most `per_snippet_chars`.
pub fn shortlist(index: &ProjectIndex, query: &str, n: usize, per_snippet_chars: usize) -> Vec<Snippet> {
    let weights = query_weights(query);
    if weights.is_empty() || index.files.is_empty() || n == 0 {
        return vec![];
    }
    let mut scored: Vec<(f64, usize)> = (0..index.files.len())
        .filter_map(|i| {
            let score = file_score(index, i, &weights);
            (score > 0.0).then_some((score, i))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored
        .into_iter()
        .take(n)
        .map(|(_, i)| snippet(&index.files[i].rel, &index.files[i].content, &weights, per_snippet_chars))
        .collect()
}

/// TF-IDF relevance of file `i` to the weighted query terms; 0 means unrelated.
pub(crate) fn file_score(index: &ProjectIndex, i: usize, weights: &HashMap<String, f64>) -> f64 {
    let n = index.files.len() as f64;
    let f = &index.files[i];
    let path_lower = f.rel.to_lowercase();
    weights
        .iter()
        .map(|(t, w)| {
            let idf = (1.0 + n / f64::from(*index.doc_freq.get(t).unwrap_or(&0)).max(1.0)).ln();
            let tf = f.terms.get(t).copied().unwrap_or(0);
            let body = if tf > 0 { 1.0 + f64::from(tf).ln() } else { 0.0 };
            let path = if path_lower.contains(t.as_str()) { 2.0 } else { 0.0 };
            w * idf * (body + path)
        })
        .sum()
}

pub(crate) fn query_weights(query: &str) -> HashMap<String, f64> {
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
    window(path, &lines, best_line(&lines, weights), max_chars)
}

/// Index of the line with the most (weighted) query hits; the first line if none.
pub(crate) fn best_line(lines: &[&str], weights: &HashMap<String, f64>) -> usize {
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let lower = l.to_lowercase();
            let hits: f64 = weights.iter().filter(|(t, _)| lower.contains(t.as_str())).map(|(_, w)| w).sum();
            (hits, i)
        })
        .fold((0.0, 0), |acc, x| if x.0 > acc.0 { x } else { acc })
        .1
}

/// Lines around `center` (8 before, 16 after), cut to `max_chars`.
pub(crate) fn window(path: &str, lines: &[&str], center: usize, max_chars: usize) -> Snippet {
    let start = center.saturating_sub(WINDOW_BEFORE);
    let end = (center + WINDOW_AFTER).min(lines.len());
    let mut text = lines[start.min(end)..end].join("\n");
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

    #[test]
    fn shortlist_keeps_keyword_order_and_clips_each_snippet() {
        let dir = fixture();
        let idx = scan(dir.path()).unwrap();
        let hits = shortlist(&idx, "I want invoices to include discounts per cart", 20, 40);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].path, "src/billing/invoice.ts");
        assert!(hits.iter().all(|s| s.text.len() <= 40));
        assert_eq!(retrieve(&idx, "I want invoices to include discounts per cart", 2, 80), shortlist(&idx, "I want invoices to include discounts per cart", 2, 40));
    }
}
