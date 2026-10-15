//! Claude via the Messages API with the user's own API key.
//! This is the path to use if you ever distribute Live Spec to other people.

use serde_json::{json, Value};

use super::{sse, ChatRequest, Chunk, OnChunk, Secret, Usage};
use crate::settings::Effort;
use crate::usage::price;
use crate::{Error, Result};

const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone)]
pub struct AnthropicApi {
    pub api_key: Secret,
    pub model: String,
    /// `output_config.effort`; None leaves the model's default.
    pub effort: Option<Effort>,
    pub http: reqwest::Client,
}

/// The request body. `output_config` only when an effort is set, so "Default"
/// sends exactly what Live Spec sent before effort existed.
fn body(model: &str, effort: Option<Effort>, req: &ChatRequest) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": req.max_tokens,
        "stream": true,
        "system": req.system,
        "messages": [{ "role": "user", "content": req.user }],
    });
    if let Some(e) = effort {
        body["output_config"] = json!({ "effort": e.as_str() });
    }
    body
}

/// Usage spread over the stream: input and cache counts in `message_start`,
/// the running output count in each `message_delta` (the last one is final).
#[derive(Debug, Default)]
struct UsageAcc {
    model: Option<String>,
    input: u64,
    cache_write: u64,
    cache_read: u64,
    output: u64,
    seen: bool,
}

impl UsageAcc {
    fn observe(&mut self, v: &Value) {
        let n = |x: &Value| x.as_u64().unwrap_or(0);
        match v["type"].as_str() {
            Some("message_start") => {
                let (message, u) = (&v["message"], &v["message"]["usage"]);
                self.model = message["model"].as_str().map(str::to_owned);
                self.input = n(&u["input_tokens"]);
                self.cache_write = n(&u["cache_creation_input_tokens"]);
                self.cache_read = n(&u["cache_read_input_tokens"]);
                self.output = n(&u["output_tokens"]);
                self.seen = true;
            }
            Some("message_delta") => {
                if let Some(output) = v["usage"]["output_tokens"].as_u64() {
                    self.output = output;
                    self.seen = true;
                }
            }
            _ => {}
        }
    }

    /// None when the stream never reported usage. Priced from `usage::PRICES`; a response model the table
    /// doesn't know falls back to the requested id when that one is listed.
    fn usage(self, requested: &str) -> Option<Usage> {
        if !self.seen {
            return None;
        }
        // The response may name a dated snapshot ("claude-haiku-4-5-20251001") that the price
        // table lacks while the id we asked for is listed: price and file it under the latter.
        let model = match self.model {
            Some(m) if price(&m).is_some() || price(requested).is_none() => m,
            _ => requested.to_owned(),
        };
        let mut u = Usage {
            model,
            input: self.input,
            cache_write: self.cache_write,
            cache_read: self.cache_read,
            output: self.output,
            cost_usd: None,
        };
        u.cost_usd = price(&u.model).map(|p| p.cost(&u));
        Some(u)
    }
}

impl AnthropicApi {
    pub async fn stream(&self, req: &ChatRequest, on_chunk: OnChunk<'_>) -> Result<String> {
        let resp = self
            .http
            .post(ENDPOINT)
            .header("x-api-key", self.api_key.expose())
            .header("anthropic-version", API_VERSION)
            .json(&body(&self.model, self.effort, req))
            .send()
            .await?;
        let resp = sse::ensure_success(resp, "Anthropic API").await?;

        let mut full = String::new();
        let mut acc = UsageAcc::default();
        let streamed = sse::for_each_data(resp, |data| {
            let v: Value = serde_json::from_str(data)?;
            acc.observe(&v);
            match v["type"].as_str() {
                Some("content_block_delta") if v["delta"]["type"] == "text_delta" => {
                    if let Some(t) = v["delta"]["text"].as_str() {
                        full.push_str(t);
                        on_chunk(Chunk::Text(t.to_owned()));
                    }
                    Ok(true)
                }
                Some("message_stop") => Ok(false),
                Some("error") => Err(Error::Provider(format!(
                    "Anthropic API stream error: {}",
                    v["error"]["message"].as_str().unwrap_or("unknown")
                ))),
                _ => Ok(true),
            }
        })
        .await;
        // Even after a stream error: the input was already billed.
        if let Some(u) = acc.usage(&self.model) {
            on_chunk(Chunk::Usage(u));
        }
        streamed?;
        Ok(full)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ChatRequest {
        ChatRequest { system: "sys".into(), user: "u".into(), max_tokens: 900, cwd: None, allow_repo_tools: false, max_turns: None }
    }

    #[test]
    fn effort_goes_in_output_config_only_when_set() {
        let plain = body("claude-haiku-5-5", None, &request());
        assert!(plain.get("output_config").is_none(), "Default sends exactly the body sent before effort existed");
        assert_eq!(plain["model"], "claude-haiku-5-5");
        assert_eq!(plain["system"], "sys");
        assert_eq!(plain["max_tokens"], 900);
        let low = body("claude-haiku-5-5", Some(Effort::Low), &request());
        assert_eq!(low["output_config"], json!({"effort": "low"}));
    }

    fn observe_all(events: &[&str]) -> UsageAcc {
        let mut acc = UsageAcc::default();
        for e in events {
            acc.observe(&serde_json::from_str(e).unwrap());
        }
        acc
    }

    const START: &str = r#"{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-haiku-5-5","content":[],"stop_reason":null,"usage":{"input_tokens":1200,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":1}}}"#;
    const DELTA: &str = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#;
    const MESSAGE_DELTA: &str = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":310}}"#;

    #[test]
    fn usage_comes_from_message_start_and_the_last_message_delta() {
        let u = observe_all(&[START, DELTA, MESSAGE_DELTA]).usage("haiku").expect("usage was reported");
        assert_eq!((u.model.as_str(), u.input, u.cache_write, u.cache_read, u.output), ("claude-haiku-5-5", 1200, 0, 0, 310));
        assert!((u.cost_usd.unwrap() - 0.000275).abs() < 1e-15, "1200 × $0.10 + 310 × $0.50 per million");
    }

    #[test]
    fn a_stream_without_usage_reports_none() {
        assert_eq!(observe_all(&[DELTA]).usage("claude-haiku-5-5"), None);
    }

    #[test]
    fn a_dated_snapshot_id_is_priced_and_filed_as_the_requested_model() {
        let start = START.replace("claude-haiku-5-5", "claude-haiku-4-5-20251001");
        let u = observe_all(&[start.as_str(), MESSAGE_DELTA]).usage("claude-haiku-4-5").unwrap();
        assert_eq!((u.model.as_str(), u.input, u.output), ("claude-haiku-4-5", 1200, 310));
        assert!((u.cost_usd.unwrap() - (1200.0 * 1.00 + 310.0 * 5.00) / 1e6).abs() < 1e-15, "priced as the requested model");
    }

    #[test]
    fn a_dated_snapshot_of_an_unpriced_model_stays_unpriced_under_the_response_id() {
        let start = START.replace("claude-haiku-5-5", "claude-sonnet-9-20270101");
        let u = observe_all(&[start.as_str(), MESSAGE_DELTA]).usage("claude-sonnet-9").unwrap();
        assert_eq!((u.model.as_str(), u.cost_usd), ("claude-sonnet-9-20270101", None));
    }

    #[test]
    fn an_unknown_model_is_counted_without_a_price() {
        let start = START.replace("claude-haiku-5-5", "claude-sonnet-9");
        let u = observe_all(&[start.as_str(), MESSAGE_DELTA]).usage("claude-sonnet-9").unwrap();
        assert_eq!((u.model.as_str(), u.output, u.cost_usd), ("claude-sonnet-9", 310, None));
    }
}
