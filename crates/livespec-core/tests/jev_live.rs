//! Calls the real TypeSafe API: a gate-style Noul and a check-style Choice,
//! three rounds each, printing latency so phase 2 can size its time budget.
//! Needs TYPESAFE_API_KEY in the environment or in the repo's .env.local:
//!   cargo test -p livespec-core --test jev_live -- --ignored --nocapture

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use livespec_core::analysis::{pre_phase, PreInput, PreOutcome, PRE_BUDGET};
use livespec_core::env_file;
use livespec_core::jev::{check, gate, Jev, DEFAULT_MODEL};
use livespec_core::project::{retrieve, scan, Snippet};
use serde_json::json;

fn key() -> Option<String> {
    std::env::var("TYPESAFE_API_KEY").ok().filter(|k| !k.trim().is_empty()).or_else(|| {
        env_file::read_value(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.env.local")), "TYPESAFE_API_KEY")
    })
}

#[tokio::test]
#[ignore = "calls the TypeSafe API"]
async fn noul_and_choice_round_trip() {
    let Some(key) = key() else {
        eprintln!("skipped: no TYPESAFE_API_KEY in the environment or .env.local");
        return;
    };
    let jev = Jev::new(key, DEFAULT_MODEL.into(), reqwest::Client::new());

    let gate = BTreeMap::from([("changed".to_owned(), gate::question())]);
    let drafts = json!({
        "previous_draft": "Cache the project index between launches.",
        "current_draft": "Cache the project index between launches, and keep one brief per git branch."
    });
    for round in 1..=3 {
        let t = Instant::now();
        let r = jev.ask(&drafts, &gate, Duration::from_secs(10)).await.unwrap();
        let noul = r.noul("changed").expect("a noul answer");
        assert!((0.0..=1.0).contains(&noul));
        eprintln!("gate noul   round {round}: {:>5} ms  noul={noul:.2}  model={}", t.elapsed().as_millis(), r.model);
    }

    let relation = BTreeMap::from([("relation".to_owned(), check::question())]);
    let code = Snippet {
        path: "src/billing/invoice.ts".into(),
        start_line: 1,
        text: "export function createInvoice(cartId: string) {\n  return { cartId, total: 0 };\n}".into(),
    };
    let claim_state = check::state("`createInvoice` ignores the cart's items when computing the total.", &code);
    for round in 1..=3 {
        let t = Instant::now();
        let r = jev.ask(&claim_state, &relation, Duration::from_secs(10)).await.unwrap();
        let (choice, confidence) = r.choice("relation").expect("a choice answer");
        assert!(["supports", "contradicts", "says_nothing"].contains(&choice));
        eprintln!("check choice round {round}: {:>5} ms  {choice} ({confidence:.2})", t.elapsed().as_millis());
    }
}

#[tokio::test]
#[ignore = "calls the TypeSafe API"]
async fn pre_phase_on_this_repo() {
    let Some(key) = key() else {
        eprintln!("skipped: no TYPESAFE_API_KEY in the environment or .env.local");
        return;
    };
    let jev = Jev::new(key, DEFAULT_MODEL.into(), reqwest::Client::new());
    let index = scan(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))).unwrap();
    let before = "Every pause spawns a fresh claude -p process from ClaudeCli::stream.";
    let changed = "Every pause spawns a fresh claude -p process from ClaudeCli::stream. I want to keep one process alive, reuse it across analyze calls, and kill it when replace_task aborts the old analysis.";
    let reworded = "Each pause starts a new claude -p process from ClaudeCli::stream.";

    for (label, current) in [("changed", changed), ("reworded", reworded)] {
        for round in 1..=3 {
            let t = Instant::now();
            let input = PreInput {
                draft: current,
                previous_draft: Some(before),
                index: Some(&index),
                max_files: 5,
                budget_chars: 30_000,
                jev: Some(&jev),
                gate: true,
                rerank: true,
            };
            match pre_phase(input, PRE_BUDGET).await {
                PreOutcome::Skip => eprintln!("{label:<9} round {round}: {:>5} ms  SKIP", t.elapsed().as_millis()),
                PreOutcome::Analyze { snippets, notice } => eprintln!(
                    "{label:<9} round {round}: {:>5} ms  notice={notice:?}  picked={:?}",
                    t.elapsed().as_millis(),
                    snippets.iter().map(|s| s.path.as_str()).collect::<Vec<_>>()
                ),
            }
        }
    }
    let keyword: Vec<String> = retrieve(&index, changed, 5, 30_000).into_iter().map(|s| s.path).collect();
    eprintln!("keyword order (no Jev): {keyword:?}");
}
