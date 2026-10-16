//! API keys, stored in the OS credential store (Windows Credential Manager,
//! macOS Keychain, Secret Service on Linux). Environment variables such as
//! `OPENAI_API_KEY` are used when no stored key exists.
use std::collections::HashMap;
use std::path::Path;

const SERVICE: &str = "PromptMan";

fn entry(provider: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, provider).map_err(|e| e.to_string())
}

fn env_var(provider: &str) -> String {
    format!("{}_API_KEY", provider.to_uppercase())
}

/// The key saved in the OS keychain, ignoring environment variables.
fn stored_key(provider: &str) -> Option<String> {
    entry(provider).ok().and_then(|e| e.get_password().ok()).filter(|k| !k.trim().is_empty())
}

/// Stored key, else the `<PROVIDER>_API_KEY` environment variable.
pub fn get_key(provider: &str) -> Option<String> {
    stored_key(provider).or_else(|| std::env::var(env_var(provider)).ok().filter(|k| !k.trim().is_empty()))
}

/// Where the key comes from, for display.
pub fn key_source(provider: &str) -> Option<&'static str> {
    if stored_key(provider).is_some() {
        Some("Saved in OS keychain")
    } else if std::env::var(env_var(provider)).is_ok_and(|k| !k.trim().is_empty()) {
        Some("From environment variable")
    } else {
        None
    }
}

pub fn set_key(provider: &str, key: &str) -> Result<(), String> {
    entry(provider)?.set_password(key.trim()).map_err(|e| e.to_string())
}

pub fn delete_key(provider: &str) -> Result<(), String> {
    match entry(provider)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Moves keys out of the pre-1.0 `vault.enc` file into the OS keychain, then
/// deletes the file. That file used a predictable key and a fixed nonce, so it
/// must not be left on disk once its contents are safely migrated.
pub fn migrate_legacy_vault(data_dir: &Path) -> Result<usize, String> {
    let path = data_dir.join("vault.enc");
    if !path.exists() {
        return Ok(0);
    }
    let keys = legacy::decrypt(&path)?;
    for (provider, key) in &keys {
        // A key saved since then is newer than the vault's copy; keep it.
        if stored_key(provider).is_some() {
            continue;
        }
        set_key(provider, key)?;
        // Read back from the keychain only, so an environment variable can't mask a failed write.
        if stored_key(provider).as_deref() != Some(key.trim()) {
            return Err(format!("could not verify migrated key for {provider}"));
        }
    }
    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    Ok(keys.len())
}

mod legacy {
    use super::*;
    use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};

    #[derive(serde::Deserialize)]
    struct VaultFile {
        nonce_hex: String,
        ciphertext_hex: String,
    }

    /// Reproduces the old key derivation so existing keys can be read once.
    fn legacy_key() -> [u8; 32] {
        let user = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "promptman_user".to_string());
        let seed = format!("{user}:{}", std::env::consts::OS);
        let mut key = [0u8; 32];
        for (i, b) in seed.as_bytes().iter().enumerate() {
            key[i % 32] ^= b;
        }
        for (i, k) in key.iter_mut().enumerate() {
            *k = k.wrapping_add((i as u8).wrapping_mul(31));
        }
        key
    }

    fn hex(s: &str) -> Result<Vec<u8>, String> {
        // ASCII check first: slicing a multi-byte character would panic.
        if !s.is_ascii() || !s.len().is_multiple_of(2) {
            return Err("invalid hex".into());
        }
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string())).collect()
    }

    pub fn decrypt(path: &Path) -> Result<HashMap<String, String>, String> {
        let file: VaultFile = serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let cipher = Aes256Gcm::new_from_slice(&legacy_key()).map_err(|e| e.to_string())?;
        let nonce = hex(&file.nonce_hex)?;
        // `Nonce::from_slice` panics on any other length.
        if nonce.len() != 12 {
            return Err("old vault file is damaged (bad nonce)".to_string());
        }
        let plain = cipher
            .decrypt(Nonce::from_slice(&nonce), hex(&file.ciphertext_hex)?.as_ref())
            .map_err(|_| "old vault could not be decrypted".to_string())?;
        serde_json::from_slice(&plain).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::legacy;

    fn vault_file(name: &str, json: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("promptman-vault-{}-{name}.enc", std::process::id()));
        std::fs::write(&path, json).unwrap();
        path
    }

    #[test]
    fn empty_environment_keys_do_not_count() {
        // A provider name no real keychain entry uses; only this test touches its variable.
        let provider = "promptmantestprovider";
        std::env::set_var("PROMPTMANTESTPROVIDER_API_KEY", "  ");
        assert_eq!(super::get_key(provider), None);
        assert_eq!(super::key_source(provider), None);
        std::env::set_var("PROMPTMANTESTPROVIDER_API_KEY", "sk-test");
        assert_eq!(super::get_key(provider).as_deref(), Some("sk-test"));
        assert_eq!(super::key_source(provider), Some("From environment variable"));
        std::env::remove_var("PROMPTMANTESTPROVIDER_API_KEY");
    }

    #[test]
    fn damaged_vault_is_an_error_not_a_panic() {
        for (name, json) in [
            ("short-nonce", r#"{"nonce_hex":"0001","ciphertext_hex":"00"}"#),
            ("non-ascii", r#"{"nonce_hex":"é0","ciphertext_hex":"00"}"#),
            ("odd-length", r#"{"nonce_hex":"000","ciphertext_hex":"00"}"#),
            ("not-json", "garbage"),
        ] {
            let path = vault_file(name, json);
            assert!(legacy::decrypt(&path).is_err(), "{name} should fail cleanly");
            std::fs::remove_file(path).unwrap();
        }
    }
}
