//! TypeSafe Jev: fast, typed judgments (a yes-probability, or one option out
//! of a set) instead of generated text. One request carries one `state` and
//! any number of independent questions about it.
//! API reference: https://docs.typesafe.ai/api.md

pub mod check;
pub mod gate;
pub mod key;
pub mod rerank;
#[cfg(test)]
pub(crate) mod test_server;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

use crate::{Error, Result};

pub const DEFAULT_MODEL: &str = "jev-latest";
const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai";
/// Waits before the first and second retry of a 429, 503 or 529.
const RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(250), Duration::from_millis(750)];

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Question {
    Noul {
        instructions: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        criteria: Option<NoulCriteria>,
    },
    Choice { instructions: Value, criteria: Options },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoulCriteria {
    #[serde(rename = "true")]
    pub yes: Value,
    #[serde(rename = "false")]
    pub no: Value,
}

/// Choice options, serialized as a JSON object in the order given. Jev 1.13
/// can lean towards the first option, so the order is a decision.
#[derive(Debug, Clone, PartialEq)]
pub struct Options(pub Vec<(String, Value)>);

impl Serialize for Options {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (option, description) in &self.0 {
            map.serialize_entry(option, description)?;
        }
        map.end()
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Answer {
    Noul { noul: f64 },
    Choice { choice: String, probabilities: BTreeMap<String, f64>, confidence: f64 },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Response {
    /// The versioned model that answered (e.g. `jev-1.13.0`), worth logging.
    pub model: String,
    pub answers: BTreeMap<String, Answer>,
}

impl Response {
    pub fn noul(&self, id: &str) -> Option<f64> {
        match self.answers.get(id)? {
            Answer::Noul { noul } => Some(*noul),
            Answer::Choice { .. } => None,
        }
    }

    /// The chosen option and its confidence.
    pub fn choice(&self, id: &str) -> Option<(&str, f64)> {
        match self.answers.get(id)? {
            Answer::Choice { choice, confidence, .. } => Some((choice.as_str(), *confidence)),
            Answer::Noul { .. } => None,
        }
    }
}

#[derive(Serialize)]
struct Request<'a> {
    state: &'a Value,
    model: &'a str,
    questions: &'a BTreeMap<String, Question>,
}

#[derive(Debug, Clone)]
pub struct Jev {
    pub api_key: String,
    pub model: String,
    pub http: reqwest::Client,
    /// `https://api.typesafe.ai`; tests point it at a local server.
    pub base_url: String,
}

impl Jev {
    pub fn new(api_key: String, model: String, http: reqwest::Client) -> Self {
        Self { api_key, model, http, base_url: DEFAULT_BASE_URL.to_owned() }
    }

    /// Asks every question about `state` in one request. Retries 429/503/529
    /// twice and never runs past `timeout` in total.
    pub async fn ask(&self, state: &Value, questions: &BTreeMap<String, Question>, timeout: Duration) -> Result<Response> {
        let deadline = Instant::now() + timeout;
        let url = format!("{}/v1/systemone", self.base_url.trim_end_matches('/'));
        let body = Request { state, model: &self.model, questions };
        let mut retries = 0;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(timed_out());
            }
            let resp = self
                .http
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body)
                .timeout(remaining)
                .send()
                .await
                .map_err(|e| if e.is_timeout() { timed_out() } else { Error::Http(e) })?;
            if resp.status().is_success() {
                return Ok(resp.json::<Response>().await?);
            }
            let status = resp.status().as_u16();
            if status == 401 {
                return Err(Error::Precondition("TypeSafe key rejected. Check .env.local or Settings → Jev.".into()));
            }
            if matches!(status, 429 | 503 | 529) && retries < RETRY_DELAYS.len() {
                let wait = retry_after(&resp).unwrap_or(RETRY_DELAYS[retries]);
                if Instant::now() + wait < deadline {
                    tokio::time::sleep(wait).await;
                    retries += 1;
                    continue;
                }
            }
            let detail: String = resp.text().await.unwrap_or_default().chars().take(300).collect();
            return Err(Error::Provider(format!("Jev returned {status}: {detail}")));
        }
    }
}

fn timed_out() -> Error {
    Error::Provider("Jev timed out".into())
}

fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
    let secs: f64 = resp.headers().get(reqwest::header::RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs_f64(secs))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use serde_json::json;

    use super::test_server::{serve, Canned};
    use super::*;

    const NOUL_OK: &str = r#"{"model":"jev-1.13.0","answers":{"is_urgent":{"type":"noul","noul":0.95}},"usage":{"input_tokens":307,"output_tokens":20}}"#;

    fn urgent() -> BTreeMap<String, Question> {
        BTreeMap::from([(
            "is_urgent".to_owned(),
            Question::Noul {
                instructions: json!("Does this convey urgency?"),
                criteria: Some(NoulCriteria { yes: json!("Explicitly time-sensitive"), no: json!("No urgency expressed") }),
            },
        )])
    }

    fn client(base_url: String) -> Jev {
        Jev { base_url, ..Jev::new("test-key".into(), DEFAULT_MODEL.into(), reqwest::Client::new()) }
    }

    #[test]
    fn noul_request_matches_the_api_reference() {
        let state = json!("Help! My payouts have been failing for 3 days.");
        let questions = urgent();
        let request = Request { state: &state, model: "jev-latest", questions: &questions };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({
                "state": "Help! My payouts have been failing for 3 days.",
                "model": "jev-latest",
                "questions": {"is_urgent": {
                    "type": "noul",
                    "instructions": "Does this convey urgency?",
                    "criteria": {"true": "Explicitly time-sensitive", "false": "No urgency expressed"}
                }}
            })
        );
    }

    #[test]
    fn choice_options_keep_their_order() {
        let q = Question::Choice {
            instructions: json!("Which team should handle this?"),
            criteria: Options(vec![
                ("sales".into(), json!("Pricing")),
                ("billing".into(), json!("Payments")),
                ("technical".into(), Value::Null),
            ]),
        };
        let text = serde_json::to_string(&q).unwrap();
        let pos = |s: &str| text.find(s).unwrap();
        assert!(pos("\"sales\"") < pos("\"billing\"") && pos("\"billing\"") < pos("\"technical\""), "{text}");
        assert_eq!(serde_json::to_value(&q).unwrap()["criteria"]["technical"], Value::Null);
    }

    #[test]
    fn parses_reference_answers() {
        let noul: Response = serde_json::from_str(NOUL_OK).unwrap();
        assert_eq!(noul.noul("is_urgent"), Some(0.95));
        let choice: Response = serde_json::from_str(
            r#"{"model":"jev-1.13.0","answers":{"department":{"type":"choice","choice":"billing","probabilities":{"billing":0.88,"technical":0.12,"sales":0.0},"confidence":0.81}},"usage":{"input_tokens":318,"output_tokens":34}}"#,
        )
        .unwrap();
        assert_eq!(choice.choice("department"), Some(("billing", 0.81)));
        assert_eq!(choice.noul("department"), None);
    }

    #[tokio::test]
    async fn retries_overload_then_answers() {
        let (url, hits) = serve(vec![Canned::new(529, "{}"), Canned::new(200, NOUL_OK)]).await;
        let r = client(url).ask(&json!("x"), &urgent(), Duration::from_secs(5)).await.unwrap();
        assert_eq!(r.noul("is_urgent"), Some(0.95));
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn retries_a_momentarily_unavailable_model() {
        let busy = r#"{"detail":{"error_type":"model_unavailable","message":"The model is unavailable."}}"#;
        let (url, hits) = serve(vec![Canned::new(503, busy), Canned::new(200, NOUL_OK)]).await;
        let r = client(url).ask(&json!("x"), &urgent(), Duration::from_secs(5)).await.unwrap();
        assert_eq!(r.noul("is_urgent"), Some(0.95));
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn gives_up_after_two_retries() {
        let (url, hits) = serve(vec![Canned::new(429, "slow down"); 3]).await;
        let err = client(url).ask(&json!("x"), &urgent(), Duration::from_secs(5)).await.unwrap_err();
        assert!(err.to_string().contains("Jev returned 429"), "{err}");
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_rejected_key_is_a_precondition() {
        let (url, _) = serve(vec![Canned::new(401, "{}")]).await;
        let err = client(url).ask(&json!("x"), &urgent(), Duration::from_secs(5)).await.unwrap_err();
        assert!(matches!(err, Error::Precondition(_)), "{err}");
    }
}
