//! File helpers shared by the stores that keep JSON under the app's folders
//! (instructions, sessions, usage): atomic writes, and keeping a file that
//! doesn't parse instead of overwriting it, because it may hold the user's work.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::Result;

/// What `load_json` found.
#[derive(Debug)]
pub enum Loaded<T> {
    /// No file: a first run.
    Missing,
    Parsed(T),
    /// The file didn't parse. It was renamed to `kept`; `None` if even that failed.
    Corrupt { kept: Option<PathBuf> },
}

/// Atomic write: `<file>.tmp` then rename; creates the parent dir.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = with_suffix(path, ".tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Renames a corrupt file to the first free `<name>.bad`, `<name>.bad.1`, …;
/// never replaces an earlier copy.
pub fn set_aside(path: &Path) -> Option<PathBuf> {
    let kept = (0u32..)
        .map(|n| with_suffix(path, &if n == 0 { ".bad".to_owned() } else { format!(".bad.{n}") }))
        .find(|p| !p.exists())?;
    std::fs::rename(path, &kept).ok()?;
    Some(kept)
}

/// Err only for read errors other than NotFound (those files are left in place).
pub fn load_json<T: DeserializeOwned>(path: &Path) -> Result<Loaded<T>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(match serde_json::from_slice(&bytes) {
            Ok(value) => Loaded::Parsed(value),
            Err(_) => Loaded::Corrupt { kept: set_aside(path) },
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Loaded::Missing),
        Err(e) => Err(e.into()),
    }
}

/// `doc.json` + `.tmp` = `doc.json.tmp`.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Deserialize)]
    struct Doc {
        n: u32,
    }

    #[test]
    fn writes_atomically_into_a_new_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/doc.json");
        write_atomic(&path, br#"{"n":1}"#).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"{"n":1}"#);
        assert!(!dir.path().join("a/b/doc.json.tmp").exists(), "the temporary file is renamed, not left behind");
        write_atomic(&path, br#"{"n":2}"#).unwrap();
        assert!(matches!(load_json::<Doc>(&path).unwrap(), Loaded::Parsed(Doc { n: 2 })));
    }

    #[test]
    fn a_missing_file_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(load_json::<Doc>(&dir.path().join("nope.json")).unwrap(), Loaded::Missing));
    }

    #[test]
    fn a_corrupt_file_is_kept_and_earlier_copies_are_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.json");
        for (i, body) in ["first", "second", "third"].iter().enumerate() {
            std::fs::write(&path, body).unwrap();
            let Loaded::Corrupt { kept: Some(kept) } = load_json::<Doc>(&path).unwrap() else { panic!("not kept") };
            let name = if i == 0 { "doc.json.bad".to_owned() } else { format!("doc.json.bad.{i}") };
            assert_eq!(kept, dir.path().join(name));
            assert!(!path.exists(), "the damaged file moved aside");
        }
        assert_eq!(std::fs::read_to_string(dir.path().join("doc.json.bad")).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(dir.path().join("doc.json.bad.1")).unwrap(), "second");
        assert_eq!(std::fs::read_to_string(dir.path().join("doc.json.bad.2")).unwrap(), "third");
    }

    #[test]
    fn a_file_that_cannot_be_read_is_an_error_and_left_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.json");
        // A directory where the file should be: reading fails, but not with NotFound.
        std::fs::create_dir(&path).unwrap();
        assert!(load_json::<Doc>(&path).is_err());
        assert!(path.is_dir(), "nothing was renamed");
        assert!(!dir.path().join("doc.json.bad").exists(), "a read failure is not corruption");
    }
}
