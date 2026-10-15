//! Live Spec core: everything that doesn't need Tauri.
//!
//! Keeping this crate UI-agnostic means the expensive logic (project indexing,
//! retrieval, prompting, provider streaming, local runtime management) is
//! unit-testable with plain `cargo test` and reusable from a CLI later.

pub mod agents;
pub mod analysis;
pub mod env_file;
pub mod error;
pub mod instructions;
pub mod jev;
pub mod llm;
pub mod local;
pub mod menu;
pub mod migrate;
pub mod project;
pub mod prompt;
pub mod sections;
pub mod sessions;
pub mod settings;
pub mod shell_env;
pub mod store;
pub mod usage;

pub use error::{Error, Result};
