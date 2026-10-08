//! OpenAI-compatible chat completions. Used for the managed llama.cpp server,
//! and works unchanged against Ollama or LM Studio if you add them later.

use serde_json::{json, Value};

use super::{sse, ChatRequest, Chunk, OnChunk};
use crate::Result;

#[derive(Debug, Clone)]
pub struct OpenAiCompat {
    /// e.g. `http://127.0.0.1:51234`
    pub base_url: String,
    pub model: String,
    pub context_tokens: u32,
    pub http: reqwest::Client,
}

impl OpenAiCompat {
    pub async fn stream(&self, req: &ChatRequest, on_chunk: OnChunk<'_>) -> Result<String> {
        let body = json!({
            "model": self.model,
            "stream": true,
            "max_tokens": req.max_tokens,
            "temperature": 0.4,
            "messages": [
                { "role": "system", "content": req.system },
                { "role": "user", "content": req.user },
            ],
        });
        let url = format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'));
        let resp = self.http.post(url).json(&body).send().await?;
        let resp = sse::ensure_success(resp, "Local model").await?;

        let mut full = String::new();
        sse::for_each_data(resp, |data| {
            if data.trim() == "[DONE]" {
                return Ok(false);
            }
            let v: Value = serde_json::from_str(data)?;
            if let Some(t) = v["choices"][0]["delta"]["content"].as_str() {
                if !t.is_empty() {
                    full.push_str(t);
                    on_chunk(Chunk::Text(t.to_owned()));
                }
            }
            Ok(true)
        })
        .await?;
        Ok(full)
    }
}
