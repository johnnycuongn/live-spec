//! The gate: does this edit change the idea, or only the wording? A pause
//! whose draft says the same thing as the last analysed one skips the model
//! call and keeps the notes.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{json, Value};

use super::{Jev, NoulCriteria, Question};
use crate::prompt::shape_draft_to;
use crate::{Error, Result};

/// Skip only when Jev is fairly sure nothing changed.
pub const GATE_SKIP_BELOW: f64 = 0.3;
const DRAFT_CHARS: usize = 12_000;

pub fn question() -> Question {
    Question::Noul {
        instructions: json!(
            "Compared with `previous_draft`, does `current_draft` add, remove, or change an idea, goal, constraint, or question?"
        ),
        criteria: Some(NoulCriteria {
            yes: json!("At least one idea, goal, constraint, or question was added, removed, or changed in meaning."),
            no: json!("Only wording, spelling, punctuation, or formatting changed; the ideas are the same."),
        }),
    }
}

pub fn state(previous: &str, current: &str) -> Value {
    json!({
        "previous_draft": shape_draft_to(previous, DRAFT_CHARS),
        "current_draft": shape_draft_to(current, DRAFT_CHARS),
    })
}

/// True when Jev's yes-probability ("it changed") is low enough to skip.
pub fn unchanged(noul: f64) -> bool {
    noul < GATE_SKIP_BELOW
}

/// Asks Jev whether `current` changes the idea of `previous`. `Ok(true)`
/// means skip; on an error the caller analyses anyway.
pub async fn is_unchanged(jev: &Jev, previous: &str, current: &str, timeout: Duration) -> Result<bool> {
    let questions = BTreeMap::from([("changed".to_owned(), question())]);
    let response = jev.ask(&state(previous, current), &questions, timeout).await?;
    let noul = response.noul("changed").ok_or_else(|| Error::Provider("Jev returned an unexpected answer.".into()))?;
    Ok(unchanged(noul))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::jev::test_server::{serve, Canned};
    use crate::jev::DEFAULT_MODEL;
    use crate::Error;

    fn judge(base_url: String) -> Jev {
        Jev { base_url, ..Jev::new("test-key".into(), DEFAULT_MODEL.into(), reqwest::Client::new()) }
    }

    #[test]
    fn question_is_worded_as_specified() {
        assert_eq!(
            serde_json::to_value(question()).unwrap(),
            json!({
                "type": "noul",
                "instructions": "Compared with `previous_draft`, does `current_draft` add, remove, or change an idea, goal, constraint, or question?",
                "criteria": {
                    "true": "At least one idea, goal, constraint, or question was added, removed, or changed in meaning.",
                    "false": "Only wording, spelling, punctuation, or formatting changed; the ideas are the same."
                }
            })
        );
    }

    #[test]
    fn long_drafts_are_shaped_before_sending() {
        let long = format!("START{}END", "x".repeat(30_000));
        let s = state(&long, "short");
        let previous = s["previous_draft"].as_str().unwrap();
        assert!(previous.starts_with("START") && previous.ends_with("END") && previous.len() < 12_100);
        assert_eq!(s["current_draft"], "short");
    }

    #[test]
    fn skips_only_when_fairly_sure_nothing_changed() {
        assert!(unchanged(0.1) && unchanged(0.29));
        assert!(!unchanged(0.3) && !unchanged(0.98));
    }

    #[tokio::test]
    async fn asks_jev_and_reads_the_answer() {
        let (url, _) = serve(vec![Canned::noul("changed", 0.1), Canned::noul("changed", 0.9)]).await;
        let jev = judge(url);
        assert!(is_unchanged(&jev, "Cache the index.", "Cache the index!", Duration::from_secs(5)).await.unwrap());
        assert!(!is_unchanged(&jev, "Cache the index.", "Cache the index per branch.", Duration::from_secs(5)).await.unwrap());
    }

    #[tokio::test]
    async fn errors_reach_the_caller() {
        let (url, _) = serve(vec![Canned::new(401, "{}")]).await;
        let err = is_unchanged(&judge(url), "a", "b", Duration::from_secs(5)).await.unwrap_err();
        assert!(matches!(err, Error::Precondition(_)), "{err}");
    }
}
