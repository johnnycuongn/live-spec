//! OpenAI-compatible chat completions. Used for the managed llama.cpp server,
//! and works unchanged against Ollama or LM Studio if you add them later.

use serde_json::{json, Value};

use super::{sse, ChatRequest, Chunk, OnChunk, Usage};
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
            // The final chunk then carries `usage` (pre-implementation check 3).
            "stream_options": { "include_usage": true },
            "messages": [
                { "role": "system", "content": req.system },
                { "role": "user", "content": req.user },
            ],
        });
        let url = format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'));
        let resp = self.http.post(url).json(&body).send().await?;
        let resp = sse::ensure_success(resp, "Local model").await?;

        let mut full = String::new();
        let mut usage = None;
        let streamed = sse::for_each_data(resp, |data| {
            if data.trim() == "[DONE]" {
                return Ok(false);
            }
            let v: Value = serde_json::from_str(data)?;
            if let Some(u) = usage_of(&v, &self.model) {
                usage = Some(u); // cumulative: the last report wins
            }
            if let Some(t) = v["choices"][0]["delta"]["content"].as_str() {
                if !t.is_empty() {
                    full.push_str(t);
                    on_chunk(Chunk::Text(t.to_owned()));
                }
            }
            Ok(true)
        })
        .await;
        if let Some(u) = usage {
            on_chunk(Chunk::Usage(u));
        }
        streamed?;
        Ok(full)
    }
}

/// A chunk's `usage` object (llama.cpp puts it on the final chunk, OpenAI on a
/// chunk with no choices; both send `null` or nothing elsewhere). Filed under
/// the model we asked for, so exact and estimated runs share a bucket. Local
/// models cost nothing.
fn usage_of(v: &Value, model: &str) -> Option<Usage> {
    let u = v.get("usage").filter(|u| u.is_object())?;
    Some(Usage {
        model: model.to_owned(),
        input: u["prompt_tokens"].as_u64().unwrap_or(0),
        cache_write: 0,
        cache_read: 0,
        output: u["completion_tokens"].as_u64().unwrap_or(0),
        cost_usd: Some(0.0),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::jev::test_server::{serve_with, Canned};

    /// llama.cpp style: usage rides on the final chunk with `finish_reason`.
    const LLAMA: &str = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hel\"}}]}\n\n\
        data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"}}]}\n\n\
        data: {\"choices\":[{\"finish_reason\":\"stop\",\"index\":0,\"delta\":{}}],\"usage\":{\"completion_tokens\":2,\"prompt_tokens\":31,\"total_tokens\":33}}\n\n\
        data: [DONE]\n\n";
    /// OpenAI style: `usage: null` on every chunk, then a chunk with no choices.
    const OPENAI: &str = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"}}],\"usage\":null}\n\n\
        data: {\"choices\":[],\"usage\":{\"prompt_tokens\":31,\"completion_tokens\":2,\"total_tokens\":33}}\n\n\
        data: [DONE]\n\n";
    const NO_USAGE: &str = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"}}]}\n\ndata: [DONE]\n\n";

    /// Streams one request against `reply`; returns the answer, the chunks and the request body.
    async fn run(reply: &'static str) -> (String, Vec<Chunk>, serde_json::Value) {
        let seen = Arc::new(Mutex::new(String::new()));
        let body = seen.clone();
        let (base_url, _) = serve_with(move |req: &str| {
            *body.lock().unwrap() = req.to_owned();
            Some(Canned::new(200, reply))
        })
        .await;
        let local = OpenAiCompat { base_url, model: "qwen3-4b-instruct".into(), context_tokens: 4096, http: reqwest::Client::new() };
        let req = ChatRequest { system: "s".into(), user: "u".into(), max_tokens: 50, cwd: None, allow_repo_tools: false, max_turns: None };
        let mut chunks = Vec::new();
        let text = local.stream(&req, &mut |c: Chunk| chunks.push(c)).await.unwrap();
        let sent = serde_json::from_str(&seen.lock().unwrap()).unwrap();
        (text, chunks, sent)
    }

    fn local_usage() -> Chunk {
        Chunk::Usage(Usage { model: "qwen3-4b-instruct".into(), input: 31, cache_write: 0, cache_read: 0, output: 2, cost_usd: Some(0.0) })
    }

    #[tokio::test]
    async fn asks_for_usage_and_reports_it_once_at_the_end() {
        let (text, chunks, sent) = run(LLAMA).await;
        assert_eq!(text, "Hello");
        assert_eq!(chunks, vec![Chunk::Text("Hel".into()), Chunk::Text("lo".into()), local_usage()]);
        assert_eq!(sent["stream_options"], serde_json::json!({ "include_usage": true }));
    }

    #[tokio::test]
    async fn reads_openai_style_usage_chunks_too() {
        let (_, chunks, _) = run(OPENAI).await;
        assert_eq!(chunks, vec![Chunk::Text("Hello".into()), local_usage()]);
    }

    #[tokio::test]
    async fn a_server_that_reports_no_usage_emits_none() {
        let (text, chunks, _) = run(NO_USAGE).await;
        assert_eq!((text.as_str(), chunks), ("Hello", vec![Chunk::Text("Hello".into())]));
    }
}
