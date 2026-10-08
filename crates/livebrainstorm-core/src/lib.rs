//! LiveBrainstorm core: everything that doesn't need Tauri.
//!
//! Keeping this crate UI-agnostic means the expensive logic (project indexing,
//! retrieval, prompting, provider streaming, local runtime management) is
//! unit-testable with plain `cargo test` and reusable from a CLI later.

pub mod error;
pub mod jev;
pub mod llm;
pub mod local;
pub mod project;
pub mod prompt;
pub mod settings;
pub mod shell_env;

pub use error::{Error, Result};
