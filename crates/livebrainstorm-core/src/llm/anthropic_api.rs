//! Claude via the Messages API with the user's own API key.
//! This is the path to use if you ever distribute LiveBrainstorm to other people.

use serde_json::{json, Value};

use super::{sse, ChatRequest, Chunk, OnChunk};
use crate::{Error, Result};

const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone)]
pub struct AnthropicApi {
    pub api_key: String,
    pub model: String,
    pub http: reqwest::Client,
}

impl AnthropicApi {
    pub async fn stream(&self, req: &ChatRequest, on_chunk: OnChunk<'_>) -> Result<String> {
        let body = json!({
            "model": self.model,
            "max_tokens": req.max_tokens,
            "stream": true,
            "system": req.system,
            "messages": [{ "role": "user", "content": req.user }],
        });
        let resp = self
            .http
            .post(ENDPOINT)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .json(&body)
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
