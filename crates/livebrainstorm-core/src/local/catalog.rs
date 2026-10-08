//! Curated models. Small instruct models that fit typical laptops; Q4_K_M is
//! the usual quality/size sweet spot. Add entries here, nothing else changes.

use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    pub approx_size_gb: f32,
    /// Context we launch llama-server with (not the model's maximum: RAM).
    pub context_tokens: u32,
    pub note: &'static str,
}

pub const CATALOG: &[ModelSpec] = &[
    ModelSpec {
        id: "qwen3-4b-instruct",
        name: "Qwen3 4B Instruct",
        file: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        approx_size_gb: 2.5,
        context_tokens: 16_384,
        note: "Best default. Good reasoning for its size, runs on 8 GB RAM.",
    },
    ModelSpec {
        id: "llama-3.2-3b-instruct",
        name: "Llama 3.2 3B Instruct",
        file: "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
        url: "https://huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF/resolve/main/Llama-3.2-3B-Instruct-Q4_K_M.gguf",
        approx_size_gb: 2.0,
        context_tokens: 8_192,
        note: "Smallest and fastest. Lighter critique.",
    },
    ModelSpec {
        id: "qwen2.5-coder-7b-instruct",
        name: "Qwen2.5 Coder 7B Instruct",
        file: "Qwen2.5-Coder-7B-Instruct-Q4_K_M.gguf",
        url: "https://huggingface.co/bartowski/Qwen2.5-Coder-7B-Instruct-GGUF/resolve/main/Qwen2.5-Coder-7B-Instruct-Q4_K_M.gguf",
        approx_size_gb: 4.7,
        context_tokens: 16_384,
        note: "Strongest on code. Wants 16 GB RAM or a GPU.",
    },
];

pub fn find(id: &str) -> Option<&'static ModelSpec> {
    CATALOG.iter().find(|m| m.id == id)
}
