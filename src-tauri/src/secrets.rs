//! Secrets live in the OS keychain (Keychain / Credential Manager / Secret
//! Service), never in settings.json or the webview.

use livebrainstorm_core::jev::key::{pick_key, KeySource};
use livebrainstorm_core::{Error, Result};

const SERVICE: &str = "app.livebrainstorm.desktop";
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
    livebrainstorm_core::env_file::read_value(path, JEV_ENV)
}

#[cfg(not(debug_assertions))]
fn dev_env_file_key() -> Option<String> {
    None
}
