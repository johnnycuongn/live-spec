//! Reads one value from a dotenv file without touching the process
//! environment. Used for the repo's `.env.local` in development builds.

use std::path::Path;

/// `name`'s value in the dotenv file at `path`; None if the file, the line or
/// the value is missing. Handles `export`, quotes and comments (dotenvy);
/// Windows line endings are normalised first.
pub fn read_value(path: &Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?.replace("\r\n", "\n");
    dotenvy::from_read_iter(text.as_bytes())
        .flatten()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(contents: &str) -> Option<String> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env.local");
        std::fs::write(&path, contents).unwrap();
        read_value(&path, "TYPESAFE_API_KEY")
    }

    #[test]
    fn reads_common_dotenv_spellings() {
        assert_eq!(read("TYPESAFE_API_KEY=ts-123\n").as_deref(), Some("ts-123"));
        assert_eq!(read("export TYPESAFE_API_KEY=\"ts-123\"\r\nOTHER=1\r\n").as_deref(), Some("ts-123"));
        assert_eq!(read("# key for Jev\nTYPESAFE_API_KEY='ts-123'\n").as_deref(), Some("ts-123"));
    }

    #[test]
    fn empty_or_missing_means_no_key() {
        assert_eq!(read("TYPESAFE_API_KEY=\n"), None);
        assert_eq!(read("OTHER=1\n"), None);
        assert_eq!(read_value(Path::new("/definitely/not/here/.env.local"), "TYPESAFE_API_KEY"), None);
    }
}
