//! API key lives in the OS keychain (Keychain / Credential Manager / Secret
//! Service), never in settings.json or the webview.

use livebrainstorm_core::{Error, Result};

const SERVICE: &str = "app.livebrainstorm.desktop";
const ACCOUNT: &str = "anthropic-api-key";

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| Error::Provider(format!("Keychain unavailable: {e}")))
}

pub fn api_key() -> Result<Option<String>> {
    match entry()?.get_password() {
        Ok(k) => Ok(Some(k)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(Error::Provider(format!("Couldn't read the API key from the keychain: {e}"))),
    }
}

pub fn set_api_key(key: &str) -> Result<()> {
    let entry = entry()?;
    let result = if key.trim().is_empty() {
        match entry.delete_credential() {
            Err(keyring::Error::NoEntry) => Ok(()),
            other => other,
        }
    } else {
        entry.set_password(key.trim())
    };
    result.map_err(|e| Error::Provider(format!("Couldn't save the API key to the keychain: {e}")))
}
