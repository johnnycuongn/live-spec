//! Secrets live in the OS keychain (Keychain / Credential Manager / Secret
//! Service), never in settings.json or the webview.

use livespec_core::jev::key::{pick_key, KeySource};
use livespec_core::{Error, Result};

const SERVICE: &str = "app.livespec.desktop";
/// The service the app saved under before it became Live Spec; read once to carry keys over.
const OLD_SERVICE: &str = "app.livebrainstorm.desktop";
const ACCOUNT: &str = "anthropic-api-key";
const JEV_ACCOUNT: &str = "typesafe-api-key";
const JEV_ENV: &str = "TYPESAFE_API_KEY";

fn entry(account: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, account).map_err(|e| Error::Provider(format!("Keychain unavailable: {e}")))
}

fn keychain_get(account: &str) -> Result<Option<String>> {
    match entry(account)?.get_password() {
        Ok(k) => Ok(Some(k)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(Error::Provider(format!("Couldn't read the key from the keychain: {e}"))),
    }
}

pub fn api_key() -> Result<Option<String>> {
    keychain_get(ACCOUNT)
}

pub fn set_api_key(key: &str) -> Result<()> {
    set_secret(ACCOUNT, key, "API key")
}

/// Saves the TypeSafe key to the Keychain; an empty key removes it.
pub fn set_jev_key(key: &str) -> Result<()> {
    set_secret(JEV_ACCOUNT, key, "TypeSafe key")
}

fn set_secret(account: &str, key: &str, what: &str) -> Result<()> {
    let entry = entry(account)?;
    let result = if key.trim().is_empty() {
        match entry.delete_credential() {
            Err(keyring::Error::NoEntry) => Ok(()),
            other => other,
        }
    } else {
        entry.set_password(key.trim())
    };
    result.map_err(|e| Error::Provider(format!("Couldn't save the {what} to the keychain: {e}")))
}

/// First launch after the rename: a key saved under the old service name is
/// copied to the new one. The old entry stays. macOS may ask once to allow
/// the new app to read the old item. Never fails the launch.
pub fn migrate_from_old_service() {
    for (account, what) in [(ACCOUNT, "API key"), (JEV_ACCOUNT, "TypeSafe key")] {
        if !matches!(keychain_get(account), Ok(None)) {
            continue; // already there, or the keychain is unavailable
        }
        let old = match keyring::Entry::new(OLD_SERVICE, account).and_then(|e| e.get_password()) {
            Ok(key) => key,
            Err(keyring::Error::NoEntry) => continue,
            Err(e) => {
                eprintln!("Live Spec: couldn't read the {what} saved by LiveBrainstorm: {e}");
                continue;
            }
        };
        match set_secret(account, &old, what) {
            Ok(()) => eprintln!("Live Spec: carried the {what} over from LiveBrainstorm"),
            Err(e) => eprintln!("Live Spec: {e}"),
        }
    }
}

/// The TypeSafe key: Keychain, then `TYPESAFE_API_KEY`, then (debug builds
/// only) the repo's `.env.local`. It never crosses IPC.
pub fn jev_key() -> Option<String> {
    jev_key_with_source().map(|(key, _)| key)
}

pub fn jev_key_with_source() -> Option<(String, KeySource)> {
    pick_key(keychain_get(JEV_ACCOUNT).ok().flatten(), std::env::var(JEV_ENV).ok(), dev_env_file_key)
}

/// Release builds never read a `.env` from the source tree.
#[cfg(debug_assertions)]
fn dev_env_file_key() -> Option<String> {
    let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.env.local"));
    livespec_core::env_file::read_value(path, JEV_ENV)
}

#[cfg(not(debug_assertions))]
fn dev_env_file_key() -> Option<String> {
    None
}
