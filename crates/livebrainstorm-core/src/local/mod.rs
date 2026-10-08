//! Local models: an in-app downloaded llama.cpp runtime + GGUF models.
//!
//! Why llama.cpp downloaded at runtime (not bundled as a Tauri sidecar, not
//! Ollama): `llama-server` ships with a folder of shared libraries, which a
//! single-binary sidecar can't carry; downloading on demand also keeps the
//! installer small and users without local models never pay for it. Ollama
//! would need a separate install, which breaks "download and it just runs".

pub mod catalog;
pub mod download;
pub mod runtime;

use std::path::{Path, PathBuf};

use serde::Serialize;

pub use catalog::{ModelSpec, CATALOG};
pub use runtime::{LocalServer, LLAMA_CPP_BUILD};

/// Where local things live under the app data dir.
#[derive(Debug, Clone)]
pub struct LocalPaths {
    pub root: PathBuf,
}

impl LocalPaths {
    pub fn new(app_data: &Path) -> Self {
        Self { root: app_data.join("local") }
    }
    pub fn runtime_dir(&self) -> PathBuf {
        self.root.join("runtime")
    }
    pub fn models_dir(&self) -> PathBuf {
        self.root.join("models")
    }
    pub fn downloads_dir(&self) -> PathBuf {
        self.root.join("downloads")
    }
    pub fn log_path(&self) -> PathBuf {
        self.root.join("llama-server.log")
    }
    pub fn model_path(&self, spec: &ModelSpec) -> PathBuf {
        self.models_dir().join(spec.file)
    }
    pub fn server_binary(&self) -> Option<PathBuf> {
        runtime::find_server_binary(&self.runtime_dir())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelState {
    #[serde(flatten)]
    pub spec: &'static ModelSpec,
    pub downloaded: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalInventory {
    pub runtime_installed: bool,
    pub runtime_build: &'static str,
    pub platform_supported: bool,
    pub models: Vec<ModelState>,
}

pub fn inventory(paths: &LocalPaths) -> LocalInventory {
    LocalInventory {
        runtime_installed: paths.server_binary().is_some(),
        runtime_build: LLAMA_CPP_BUILD,
        platform_supported: runtime::asset_name().is_some(),
        models: CATALOG
            .iter()
            .map(|spec| ModelState { spec, downloaded: paths.model_path(spec).is_file() })
            .collect(),
    }
}
