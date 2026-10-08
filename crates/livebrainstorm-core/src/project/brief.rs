//! On-disk cache of project briefs, keyed by project path.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{Fnv, ProjectIndex};
use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredBrief {
    pub root: String,
    /// Project fingerprint when the brief was written.
    pub fingerprint: String,
    pub text: String,
    pub created_at: u64,
    /// Which provider wrote it, shown in the UI.
    pub source: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BriefStatus {
    Missing,
    Fresh,
    /// Files changed since. Still useful; the UI offers a rebuild.
    Stale,
}

impl BriefStatus {
    pub fn of(brief: Option<&StoredBrief>, index: &ProjectIndex) -> Self {
        match brief {
            None => Self::Missing,
            Some(b) if b.fingerprint == index.fingerprint => Self::Fresh,
            Some(_) => Self::Stale,
        }
    }
}

pub struct BriefStore {
    dir: PathBuf,
}

impl BriefStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path_for(&self, root: &Path) -> PathBuf {
        let mut h = Fnv::new();
        h.write(root.to_string_lossy().as_bytes());
        self.dir.join(format!("{}.json", h.hex()))
    }

    pub fn load(&self, root: &Path) -> Option<StoredBrief> {
        let bytes = std::fs::read(self.path_for(root)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn save(&self, brief: &StoredBrief) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.path_for(Path::new(&brief.root));
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(brief)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}
