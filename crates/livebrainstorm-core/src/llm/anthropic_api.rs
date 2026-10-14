//! Claude via the Messages API with the user's own API key.
//! This is the path to use if you ever distribute LiveBrainstorm to other people.

use serde_json::{json, Value};

use super::{sse, ChatRequest, Chunk, OnChunk, Secret};
use crate::settings::Effort;
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
/// sends exactly what LiveBrainstorm sent before effort existed.
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
        sse::for_each_data(resp, |data| {
            let v: Value = serde_json::from_str(data)?;
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
        .await?;
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
}
