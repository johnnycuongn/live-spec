//! Calls the real TypeSafe API: a gate-style Noul and a check-style Choice,
//! three rounds each, printing latency so phase 2 can size its time budget.
//! Needs TYPESAFE_API_KEY in the environment or in the repo's .env.local:
//!   cargo test -p livebrainstorm-core --test jev_live -- --ignored --nocapture

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use livebrainstorm_core::env_file;
use livebrainstorm_core::jev::{check, Jev, NoulCriteria, Question, DEFAULT_MODEL};
use livebrainstorm_core::project::Snippet;
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

    let gate = BTreeMap::from([(
        "changed".to_owned(),
        Question::Noul {
            instructions: json!("Compared with `previous_draft`, does `current_draft` add, remove, or change an idea, goal, constraint, or question?"),
            criteria: Some(NoulCriteria {
                yes: json!("At least one idea, goal, constraint, or question was added, removed, or changed in meaning."),
                no: json!("Only wording, spelling, punctuation, or formatting changed; the ideas are the same."),
            }),
        },
    )]);
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
