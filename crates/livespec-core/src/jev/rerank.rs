//! Rerank: of the keyword shortlist, which snippets are actually about what
//! the draft discusses? One Noul per snippet, in parallel; the best few go to
//! the notes model.

use std::collections::BTreeMap;
use std::time::Instant;

use futures_util::stream::{self, StreamExt};
use serde_json::{json, Value};

use super::{Jev, NoulCriteria, Question};
use crate::project::Snippet;
use crate::prompt::shape_draft_to;
use crate::{Error, Result};

pub const SHORTLIST: usize = 20;
/// The whole shortlist goes out at once, so the rerank costs about one Jev
/// round trip (~0.3 s), not three waves of 8. TypeSafe allows 80 requests/s.
const RERANK_CONCURRENCY: usize = SHORTLIST;
pub const SHORTLIST_CHARS: usize = 1_500;
const DRAFT_CHARS: usize = 4_000;

pub fn question() -> Question {
    Question::Noul {
        instructions: json!(
            "Is `code` relevant to what `draft` is discussing? Relevant means it implements, defines, configures, or directly affects something the draft talks about."
        ),
        criteria: Some(NoulCriteria {
            yes: json!("The code implements, defines, configures, or directly affects a feature, component, or behaviour the draft discusses."),
            no: json!("The code is unrelated, or only shares generic words with the draft."),
        }),
    }
}

/// `shaped_draft` is already cut down with `shape_draft_to`.
pub fn state(shaped_draft: &str, code: &Snippet) -> Value {
    json!({ "draft": shaped_draft, "code": { "path": code.path, "start_line": code.start_line, "text": code.text } })
}

/// The `keep` highest-scoring snippets (ties keep shortlist order), each
/// clipped to `per_file_chars`.
pub fn pick(candidates: Vec<Snippet>, scores: &[f64], keep: usize, per_file_chars: usize) -> Vec<Snippet> {
    let mut ranked: Vec<(f64, Snippet)> = scores.iter().copied().zip(candidates).collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    ranked
        .into_iter()
        .take(keep)
        .map(|(_, mut s)| {
            if s.text.len() > per_file_chars {
                let cut = (0..=per_file_chars).rev().find(|i| s.text.is_char_boundary(*i)).unwrap_or(0);
                s.text.truncate(cut);
            }
            s
        })
        .collect()
}

/// One relevance score per candidate, in candidate order. Fails as a whole
/// if any request fails or the deadline passes: half a ranking is worse than
/// the keyword order.
pub async fn scores(jev: &Jev, draft: &str, candidates: &[Snippet], deadline: Instant) -> Result<Vec<f64>> {
    let shaped = shape_draft_to(draft, DRAFT_CHARS);
    let questions = BTreeMap::from([("relevant".to_owned(), question())]);
    let (shaped, questions) = (&shaped, &questions);
    // Indices, not `&Snippet` items: closures over borrowed items break
    // rustc's Send check when this runs inside a spawned task.
    let mut answers = std::pin::pin!(stream::iter(0..candidates.len())
        .map(|i| async move {
            let timeout = deadline.saturating_duration_since(Instant::now());
            (i, jev.ask(&state(shaped, &candidates[i]), questions, timeout).await)
        })
        .buffer_unordered(RERANK_CONCURRENCY));
    let mut out = vec![0.0; candidates.len()];
    while let Some((i, answer)) = answers.next().await {
        out[i] = answer?.noul("relevant").ok_or_else(|| Error::Provider("Jev returned an unexpected answer.".into()))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::jev::test_server::{serve_with, Canned};
    use crate::jev::DEFAULT_MODEL;

    fn judge(base_url: String) -> Jev {
        Jev { base_url, ..Jev::new("test-key".into(), DEFAULT_MODEL.into(), reqwest::Client::new()) }
    }

    fn snip(path: &str, text: &str) -> Snippet {
        Snippet { path: path.into(), start_line: 1, text: text.into() }
    }

    #[test]
    fn question_is_worded_as_specified() {
        assert_eq!(
            serde_json::to_value(question()).unwrap(),
            json!({
                "type": "noul",
                "instructions": "Is `code` relevant to what `draft` is discussing? Relevant means it implements, defines, configures, or directly affects something the draft talks about.",
                "criteria": {
                    "true": "The code implements, defines, configures, or directly affects a feature, component, or behaviour the draft discusses.",
                    "false": "The code is unrelated, or only shares generic words with the draft."
                }
            })
        );
    }

    #[test]
    fn picks_the_best_scores_with_ties_in_shortlist_order() {
        let candidates = vec![snip("a.ts", "aaaa"), snip("b.ts", "bbbbbbbb"), snip("c.ts", "cccc"), snip("d.ts", "dddd")];
        let picked = pick(candidates, &[0.2, 0.9, 0.5, 0.5], 3, 4);
        assert_eq!(picked.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(), ["b.ts", "c.ts", "d.ts"]);
        assert_eq!(picked[0].text, "bbbb", "clipped to the per-file budget");
    }

    #[tokio::test]
    async fn scores_come_back_in_candidate_order() {
        let (url, hits) = serve_with(|body: &str| Some(Canned::noul("relevant", if body.contains("createInvoice") { 0.9 } else { 0.1 }))).await;
        let candidates = vec![snip("src/cart/cart.ts", "export class CartStore {}"), snip("src/billing/invoice.ts", "export function createInvoice() {}")];
        let got = scores(&judge(url), "invoices need discounts", &candidates, Instant::now() + Duration::from_secs(5)).await.unwrap();
        assert_eq!(got, [0.1, 0.9]);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn one_failed_request_fails_the_ranking() {
        let (url, _) = serve_with(|body: &str| Some(if body.contains("CartStore") { Canned::new(500, "boom") } else { Canned::noul("relevant", 0.9) })).await;
        let candidates = vec![snip("src/cart/cart.ts", "export class CartStore {}"), snip("src/billing/invoice.ts", "createInvoice")];
        assert!(scores(&judge(url), "invoices", &candidates, Instant::now() + Duration::from_secs(5)).await.is_err());
    }

    #[tokio::test]
    async fn the_whole_shortlist_goes_out_at_once() {
        // Slow answers, so every request sent in the first wave is still open
        // when we count: one wave of 20 is about one round trip, waves of 8 are three.
        let (url, hits) = serve_with(|_: &str| Some(Canned::noul("relevant", 0.5).delayed(1_000))).await;
        let candidates: Vec<Snippet> = (0..SHORTLIST).map(|i| snip(&format!("src/f{i}.ts"), "code")).collect();
        let jev = judge(url);
        let (result, in_flight) = tokio::join!(scores(&jev, "draft", &candidates, Instant::now() + Duration::from_secs(5)), async {
            tokio::time::sleep(Duration::from_millis(400)).await;
            hits.load(std::sync::atomic::Ordering::SeqCst)
        });
        assert!(result.is_ok());
        assert_eq!(in_flight, SHORTLIST, "all shortlisted snippets are scored in one round trip");
    }
}
