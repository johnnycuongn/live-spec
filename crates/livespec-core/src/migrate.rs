//! One-time move of the app's folders from the old bundle identifier's
//! location to the new one, on the first launch after the rename. Tauri
//! resolves the data and config folders as `<base>/<identifier>`, so the old
//! folder sits beside the new one. Nothing here deletes old data, and no
//! failure stops the launch: the caller logs and carries on.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The identifier the app shipped under before it became Live Spec.
pub const OLD_IDENTIFIER: &str = "app.livebrainstorm.desktop";

/// `<base>/<new identifier>` → `<base>/<old identifier>`. None for a path with no final component.
pub fn old_path(new_dir: &Path, old_identifier: &str) -> Option<PathBuf> {
    new_dir.file_name()?;
    Some(new_dir.with_file_name(old_identifier))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Moved {
    Renamed,
    /// The rename failed (another device, say): copied instead, the old folder left in place.
    Copied,
    Nothing,
}

/// Takes `old` as `new` when `new` is missing (or an empty folder a plugin made)
/// and `old` exists. A rename is one atomic move, so models in `local/` don't
/// get copied; when it fails, the folder is copied and the old one kept.
pub fn move_dir(old: &Path, new: &Path) -> io::Result<Moved> {
    if !old.is_dir() {
        return Ok(Moved::Nothing);
    }
    match fs::read_dir(new) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Ok(Moved::Nothing); // the new folder is in use: never overwrite it
            }
            fs::remove_dir(new)?; // empty: make room for the rename
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    if fs::rename(old, new).is_ok() {
        return Ok(Moved::Renamed);
    }
    copy_dir_all(old, new)?;
    Ok(Moved::Copied)
}

fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// For each distinct folder in `new_dirs` (data and config are one folder on
/// macOS), moves the old identifier's folder in. Every outcome but "nothing to
/// do" is reported through `log`; errors are reported, never returned.
pub fn move_app_dirs(new_dirs: &[PathBuf], old_identifier: &str, mut log: impl FnMut(String)) {
    let mut seen: Vec<&PathBuf> = Vec::new();
    for new in new_dirs {
        if seen.contains(&new) {
            continue;
        }
        seen.push(new);
        let Some(old) = old_path(new, old_identifier) else { continue };
        match move_dir(&old, new) {
            Ok(Moved::Renamed) => log(format!("moved {} to {}", old.display(), new.display())),
            Ok(Moved::Copied) => log(format!("copied {} to {} (the old folder is still there)", old.display(), new.display())),
            Ok(Moved::Nothing) => {}
            Err(e) => log(format!("couldn't move {} to {}: {e}", old.display(), new.display())),
        }
    }
}

/// Runs `f` unless `marker` exists, then writes the marker (creating its
/// folder), whatever `f` did. Returns whether `f` ran. A marker that can't be
/// written is an error after `f` has run: the caller logs it, and the next
/// launch runs `f` again. For steps like the keychain copy, which must not
/// repeat: a key the user removes after the upgrade must stay removed.
pub fn once(marker: &Path, f: impl FnOnce()) -> io::Result<bool> {
    if marker.exists() {
        return Ok(false);
    }
    f();
    if let Some(dir) = marker.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(marker, b"")?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn seed(dir: &Path) {
        fs::create_dir_all(dir.join("sessions/tabs")).unwrap();
        fs::write(dir.join("sessions/tabs/t-1.json"), "{}").unwrap();
        fs::write(dir.join("settings.json"), "{}").unwrap();
    }

    #[test]
    fn once_runs_the_first_time_only_and_leaves_its_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join("not-yet/.keychain-migrated"); // a fresh install: no config folder yet
        let mut runs = 0;
        assert!(once(&marker, || runs += 1).unwrap());
        assert!(marker.exists());
        assert!(!once(&marker, || runs += 1).unwrap(), "a later launch skips it");
        assert_eq!(runs, 1, "a key the user removed after the upgrade is never copied back");
    }

    #[test]
    fn the_old_path_swaps_the_final_component() {
        let new = Path::new("/Users/x/Library/Application Support/app.livespec.desktop");
        assert_eq!(old_path(new, OLD_IDENTIFIER), Some(PathBuf::from("/Users/x/Library/Application Support/app.livebrainstorm.desktop")));
        assert_eq!(old_path(Path::new("/"), OLD_IDENTIFIER), None);
    }

    #[test]
    fn a_missing_new_folder_takes_the_old_one_by_rename() {
        let tmp = tempfile::tempdir().unwrap();
        let (old, new) = (tmp.path().join("old"), tmp.path().join("new"));
        seed(&old);
        assert_eq!(move_dir(&old, &new).unwrap(), Moved::Renamed);
        assert!(new.join("sessions/tabs/t-1.json").exists() && new.join("settings.json").exists());
        assert!(!old.exists());
    }

    #[test]
    fn an_empty_new_folder_is_treated_as_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let (old, new) = (tmp.path().join("old"), tmp.path().join("new"));
        seed(&old);
        fs::create_dir_all(&new).unwrap(); // a plugin made it before setup ran
        assert_eq!(move_dir(&old, &new).unwrap(), Moved::Renamed);
        assert!(new.join("settings.json").exists());
    }

    #[test]
    fn an_existing_new_folder_or_a_missing_old_one_moves_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let (old, new) = (tmp.path().join("old"), tmp.path().join("new"));
        seed(&old);
        seed(&new);
        fs::write(new.join("settings.json"), r#"{"mine":true}"#).unwrap();
        assert_eq!(move_dir(&old, &new).unwrap(), Moved::Nothing);
        assert_eq!(fs::read_to_string(new.join("settings.json")).unwrap(), r#"{"mine":true}"#);
        assert!(old.exists(), "never deleted");
        assert_eq!(move_dir(&tmp.path().join("absent"), &tmp.path().join("new2")).unwrap(), Moved::Nothing);
        assert!(!tmp.path().join("new2").exists());
    }

    #[test]
    fn copy_dir_all_copies_nested_files() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
        seed(&src);
        copy_dir_all(&src, &dst).unwrap();
        assert_eq!(fs::read_to_string(dst.join("sessions/tabs/t-1.json")).unwrap(), "{}");
        assert!(src.exists());
    }

    #[test]
    fn move_app_dirs_handles_each_distinct_folder_once_and_logs() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base");
        let old = base.join(OLD_IDENTIFIER);
        seed(&old);
        let new = base.join("app.livespec.desktop");
        let mut log = Vec::new();
        move_app_dirs(&[new.clone(), new.clone()], OLD_IDENTIFIER, |m| log.push(m));
        assert!(new.join("settings.json").exists() && !old.exists());
        assert_eq!(log.len(), 1, "the same folder, given twice, is handled once: {log:?}");
        assert!(log[0].contains("moved"), "{log:?}");
        let mut again = Vec::new();
        move_app_dirs(std::slice::from_ref(&new), OLD_IDENTIFIER, |m| again.push(m));
        assert!(again.is_empty(), "a second launch says nothing");
    }
}
