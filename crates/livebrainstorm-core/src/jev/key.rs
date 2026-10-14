//! Where the TypeSafe key comes from. The shell supplies the three sources
//! (Keychain, environment, `.env.local` in debug builds); this decides which
//! one wins. `KeyStatus` is what the UI may see: the source, never the key.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KeySource {
    Keychain,
    Environment,
    EnvFile,
}

/// What `jev_status` sends to the webview. Deliberately has no key field.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeyStatus {
    pub source: Option<KeySource>,
}

/// The first non-blank key: Keychain, then the environment, then the env
/// file. `env_file` only runs when the first two have nothing.
pub fn pick_key(
    keychain: Option<String>,
    environment: Option<String>,
    env_file: impl FnOnce() -> Option<String>,
) -> Option<(String, KeySource)> {
    let present = |k: Option<String>| k.map(|k| k.trim().to_owned()).filter(|k| !k.is_empty());
    if let Some(k) = present(keychain) {
        return Some((k, KeySource::Keychain));
    }
    if let Some(k) = present(environment) {
        return Some((k, KeySource::Environment));
    }
    present(env_file()).map(|k| (k, KeySource::EnvFile))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn some(s: &str) -> Option<String> {
        Some(s.to_owned())
    }

    #[test]
    fn lookup_order_is_keychain_then_environment_then_env_file() {
        assert_eq!(pick_key(some("kc"), some("env"), || some("file")), Some(("kc".into(), KeySource::Keychain)));
        assert_eq!(pick_key(None, some("env"), || some("file")), Some(("env".into(), KeySource::Environment)));
        assert_eq!(pick_key(None, None, || some("file")), Some(("file".into(), KeySource::EnvFile)));
        assert_eq!(pick_key(None, None, || None), None);
    }

    #[test]
    fn blank_values_never_shadow_a_real_key() {
        assert_eq!(pick_key(some("  "), some(""), || some(" file-key ")), Some(("file-key".into(), KeySource::EnvFile)));
    }

    #[test]
    fn the_env_file_is_only_read_when_needed() {
        let read = Cell::new(false);
        let found = pick_key(some("kc"), None, || {
            read.set(true);
            some("file")
        });
        assert_eq!(found.map(|(_, s)| s), Some(KeySource::Keychain));
        assert!(!read.get());
    }

    #[test]
    fn status_serializes_only_the_source() {
        let status = KeyStatus { source: Some(KeySource::EnvFile) };
        assert_eq!(serde_json::to_value(&status).unwrap(), serde_json::json!({"source": "envFile"}));
        assert_eq!(serde_json::to_value(KeyStatus { source: None }).unwrap(), serde_json::json!({"source": null}));
        assert_eq!(serde_json::to_value(KeySource::Keychain).unwrap(), "keychain");
        assert_eq!(serde_json::to_value(KeySource::Environment).unwrap(), "environment");
    }
}
