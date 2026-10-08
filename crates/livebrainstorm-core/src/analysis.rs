//! The per-pause pipeline before the model call ("pre-phase"): the Jev gate
//! and the Jev rerank run at the same time under one deadline. Without Jev,
//! or when it's slow or failing, this is exactly today's keyword retrieval.

use std::time::{Duration, Instant};

use crate::jev::{gate, rerank, Jev};
use crate::project::{retrieve, shortlist, ProjectIndex, Snippet};
use crate::Error;

/// Gate and rerank share this deadline. Jev answers in about 0.3 s, and the
/// rerank's 20 requests run 8 at a time (3 waves).
pub const PRE_BUDGET: Duration = Duration::from_millis(1_500);

pub struct PreInput<'a> {
    pub draft: &'a str,
    /// The last draft that was actually analysed; None means no gate.
    pub previous_draft: Option<&'a str>,
    pub index: Option<&'a ProjectIndex>,
    /// How many snippets the notes model gets, and their total size.
    pub max_files: usize,
    pub budget_chars: usize,
    /// None without a key, or for the Local provider.
    pub jev: Option<&'a Jev>,
    pub gate: bool,
    pub rerank: bool,
}

#[derive(Debug, PartialEq)]
pub enum PreOutcome {
    /// The gate found no real change: keep the notes, make no model call.
    Skip,
    Analyze { snippets: Vec<Snippet>, notice: Option<String> },
}

pub async fn pre_phase(input: PreInput<'_>, budget: Duration) -> PreOutcome {
    let deadline = Instant::now() + budget;
    // Plain `if`s, not `Option::filter` closures: the shell awaits this inside
    // a spawned task, and borrowing closures there trip rustc's Send check.
    let gate_jev = if input.gate { input.jev } else { None };
    let rerank_jev = if input.rerank { input.jev } else { None };

    let gate_check = async {
        let (jev, previous) = (gate_jev?, input.previous_draft?);
        Some(gate::is_unchanged(jev, previous, input.draft, deadline.saturating_duration_since(Instant::now())).await)
    };
    let picking = async {
        let Some(index) = input.index else { return (vec![], None) };
        let keyword = || retrieve(index, input.draft, input.max_files, input.budget_chars);
        let Some(jev) = rerank_jev else { return (keyword(), None) };
        let candidates = shortlist(index, input.draft, rerank::SHORTLIST, rerank::SHORTLIST_CHARS);
        if candidates.len() <= input.max_files {
            return (keyword(), None);
        }
        match rerank::scores(jev, input.draft, &candidates, deadline).await {
            Ok(scores) => {
                let per_file = input.budget_chars / input.max_files.max(1);
                (rerank::pick(candidates, &scores, input.max_files, per_file), None)
            }
            Err(e) => (keyword(), Some(unavailable(&e))),
        }
    };

    tokio::pin!(gate_check, picking);
    let (mut gate_answer, mut picked) = (None, None);
    while gate_answer.is_none() || picked.is_none() {
        tokio::select! {
            answer = &mut gate_check, if gate_answer.is_none() => {
                if matches!(answer, Some(Ok(true))) {
                    return PreOutcome::Skip; // dropping `picking` cancels the rerank
                }
                gate_answer = Some(answer);
            }
            result = &mut picking, if picked.is_none() => picked = Some(result),
        }
    }
    let (snippets, rerank_notice) = picked.expect("the loop ends once both are done");
    let gate_notice = match gate_answer {
        Some(Some(Err(e))) => Some(unavailable(&e)),
        _ => None,
    };
    PreOutcome::Analyze { snippets, notice: gate_notice.or(rerank_notice) }
}

fn unavailable(e: &Error) -> String {
    format!("Jev unavailable, using plain matching: {e}")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::jev::test_server::{serve_with, Canned};
    use crate::jev::DEFAULT_MODEL;
    use crate::project::scan;
    use crate::project::scan::tests::fixture;

    const DRAFT: &str = "I want invoices to include discounts per cart";

    fn judge(base_url: String) -> Jev {
        Jev { base_url, ..Jev::new("test-key".into(), DEFAULT_MODEL.into(), reqwest::Client::new()) }
    }

    fn input<'a>(index: &'a ProjectIndex, jev: Option<&'a Jev>, previous: Option<&'a str>, max_files: usize) -> PreInput<'a> {
        PreInput { draft: DRAFT, previous_draft: previous, index: Some(index), max_files, budget_chars: 4_000, jev, gate: true, rerank: true }
    }

    fn keyword(index: &ProjectIndex, max_files: usize) -> Vec<Snippet> {
        retrieve(index, DRAFT, max_files, 4_000)
    }

    /// Gate requests carry `previous_draft`; every other request is a rerank.
    fn answers(gate: f64, relevant: fn(&str) -> f64) -> impl Fn(&str) -> Option<Canned> + Send + Sync + 'static {
        move |body: &str| {
            Some(if body.contains("previous_draft") { Canned::noul("changed", gate) } else { Canned::noul("relevant", relevant(body)) })
        }
    }

    #[tokio::test]
    async fn without_jev_it_is_todays_keyword_retrieval() {
        let idx = scan(fixture().path()).unwrap();
        let out = pre_phase(input(&idx, None, Some("old"), 1), PRE_BUDGET).await;
        assert_eq!(out, PreOutcome::Analyze { snippets: keyword(&idx, 1), notice: None });
    }

    #[tokio::test]
    async fn an_unchanged_idea_skips_the_analysis() {
        let (url, _) = serve_with(answers(0.1, |_| 0.5)).await;
        let idx = scan(fixture().path()).unwrap();
        let jev = judge(url);
        assert_eq!(pre_phase(input(&idx, Some(&jev), Some("old wording"), 1), PRE_BUDGET).await, PreOutcome::Skip);
    }

    #[tokio::test]
    async fn a_changed_idea_gets_the_reranked_code() {
        let (url, _) = serve_with(answers(0.9, |body| if body.contains("CartStore") { 0.95 } else { 0.05 })).await;
        let idx = scan(fixture().path()).unwrap();
        let jev = judge(url);
        let PreOutcome::Analyze { snippets, notice } = pre_phase(input(&idx, Some(&jev), Some("old"), 1), PRE_BUDGET).await else {
            panic!("expected Analyze");
        };
        assert_eq!(notice, None);
        assert_eq!(snippets.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(), ["src/cart/cart.ts"]);
        assert_eq!(keyword(&idx, 1)[0].path, "src/billing/invoice.ts", "keyword order alone would pick invoice.ts");
    }

    #[tokio::test]
    async fn jev_errors_fall_back_to_keywords_with_one_notice() {
        let (url, _) = serve_with(|_: &str| Some(Canned::new(500, "boom"))).await;
        let idx = scan(fixture().path()).unwrap();
        let jev = judge(url);
        let PreOutcome::Analyze { snippets, notice } = pre_phase(input(&idx, Some(&jev), Some("old"), 1), PRE_BUDGET).await else {
            panic!("an error must never skip");
        };
        assert_eq!(snippets, keyword(&idx, 1));
        assert!(notice.as_deref().is_some_and(|n| n.starts_with("Jev unavailable, using plain matching")), "{notice:?}");
    }

    #[tokio::test]
    async fn a_slow_jev_never_holds_the_notes_past_the_budget() {
        // Says "unchanged", but too late: the pause must be analysed anyway.
        let (url, _) = serve_with(|body: &str| {
            let reply = if body.contains("previous_draft") { Canned::noul("changed", 0.1) } else { Canned::noul("relevant", 0.9) };
            Some(reply.delayed(2_000))
        })
        .await;
        let idx = scan(fixture().path()).unwrap();
        let jev = judge(url);
        let started = Instant::now();
        let out = pre_phase(input(&idx, Some(&jev), Some("old"), 1), Duration::from_millis(300)).await;
        assert!(started.elapsed() < Duration::from_millis(1_000), "{:?}", started.elapsed());
        assert!(matches!(out, PreOutcome::Analyze { ref snippets, notice: Some(_) } if *snippets == keyword(&idx, 1)), "{out:?}");
    }

    #[tokio::test]
    async fn no_previous_draft_or_few_candidates_cost_no_jev_calls() {
        let (url, hits) = serve_with(answers(0.1, |_| 0.9)).await;
        let idx = scan(fixture().path()).unwrap();
        let jev = judge(url);
        // No previous draft: no gate. 3 candidates for 5 slots: no rerank.
        let out = pre_phase(input(&idx, Some(&jev), None, 5), PRE_BUDGET).await;
        assert_eq!(out, PreOutcome::Analyze { snippets: keyword(&idx, 5), notice: None });
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn switched_off_features_never_call_jev() {
        let (url, hits) = serve_with(answers(0.1, |_| 0.9)).await;
        let idx = scan(fixture().path()).unwrap();
        let jev = judge(url);
        let off = PreInput { gate: false, rerank: false, ..input(&idx, Some(&jev), Some("old"), 1) };
        assert_eq!(pre_phase(off, PRE_BUDGET).await, PreOutcome::Analyze { snippets: keyword(&idx, 1), notice: None });
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }
}
