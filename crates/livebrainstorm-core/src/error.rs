use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("file system error: {0}")]
    Io(#[from] std::io::Error),
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// The model provider failed (non-2xx, CLI exit code, stream error...).
    #[error("{0}")]
    Provider(String),
    /// Something the user must do first (open a project, download a model...).
    #[error("{0}")]
    Precondition(String),
    #[error("archive error: {0}")]
    Archive(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Errors cross the Tauri IPC boundary as plain strings the UI can show.
impl Serialize for Error {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
