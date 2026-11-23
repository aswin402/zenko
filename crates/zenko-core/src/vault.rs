use crate::constants::FILE_MODE_OWNER_ONLY;
use crate::error::{Result, ZenkoError};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct VaultEnvelope {
    version: u8,
    nonce_b64: String,
    ciphertext_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VaultStats {
    pub exists: bool,
    pub encrypted: bool,
    pub cipher: String,
    pub path: String,
    pub size_bytes: u64,
    pub key_count: usize,
    pub key_names: Vec<String>,
}

pub struct SecretVault {
    vault_path: PathBuf,
    key_path: PathBuf,
    write_lock: Mutex<()>,
}

impl SecretVault {
    pub fn open(vault_path: impl Into<PathBuf>, key_path: impl Into<PathBuf>) -> Self {
        Self {
            vault_path: vault_path.into(),
            key_path: key_path.into(),
            write_lock: Mutex::new(()),
        }
    }

    fn load_key(&self) -> Result<Zeroizing<[u8; 32]>> {
        let raw = Zeroizing::new(std::fs::read_to_string(&self.key_path).map_err(|e| {
            ZenkoError::Vault(format!("vault key missing or unreadable: {e}"))
        })?);
        let bytes = Zeroizing::new(
            B64.decode(raw.trim())
                .map_err(|e| ZenkoError::Vault(format!("corrupt vault key: {e}")))?,
        );
        match bytes.len() {
            32 => {
                let mut key = Zeroizing::new([0u8; 32]);
                key.copy_from_slice(&bytes);
                Ok(key)
            }
            _ => Err(ZenkoError::Vault("invalid vault key length".into())),
        }
    }

    fn ensure_key(&self) -> Result<Zeroizing<[u8; 32]>> {
        match self.key_path.exists() {
            true => self.load_key(),
            false => {
                if let Some(parent) = self.key_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut key = Zeroizing::new([0u8; 32]);
                rand::thread_rng().fill_bytes(&mut key[..]);
                let encoded = Zeroizing::new(B64.encode(&key[..]));
                write_owner_only_atomic(&self.key_path, encoded.as_bytes())?;
                Ok(key)
            }
        }
    }

    fn load_map(&self) -> Result<BTreeMap<String, String>> {
        match self.vault_path.exists() {
            false => Ok(BTreeMap::new()),
            true => {
                let key = self.load_key()?;
                let raw = std::fs::read_to_string(&self.vault_path)?;
                let env: VaultEnvelope = serde_json::from_str(&raw)
                    .map_err(|e| ZenkoError::Vault(format!("corrupt vault file: {e}")))?;
                match env.version {
                    1 => {}
                    v => {
                        return Err(ZenkoError::Vault(format!(
                            "unsupported vault envelope version: {v}"
                        )));
                    }
                }
                let nonce_bytes = B64
                    .decode(&env.nonce_b64)
                    .map_err(|e| ZenkoError::Vault(format!("invalid nonce: {e}")))?;
                match nonce_bytes.len() {
                    12 => {}
                    _ => return Err(ZenkoError::Vault("invalid nonce length".into())),
                }
                let ciphertext = B64
                    .decode(&env.ciphertext_b64)
                    .map_err(|e| ZenkoError::Vault(format!("invalid ciphertext: {e}")))?;
                let cipher = ChaCha20Poly1305::new(Key::from_slice(&key[..]));
                let plaintext = Zeroizing::new(
                    cipher
                        .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_ref())
                        .map_err(|_| ZenkoError::Vault("vault decryption failed".into()))?,
                );
                serde_json::from_slice(&plaintext)
                    .map_err(|e| ZenkoError::Vault(format!("invalid vault payload: {e}")))
            }
        }
    }

    fn save_map(&self, map: &BTreeMap<String, String>) -> Result<()> {
        if let Some(parent) = self.vault_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let key = self.ensure_key()?;
        let plaintext = Zeroizing::new(serde_json::to_vec(map)?);
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key[..]));
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
            .map_err(|_| ZenkoError::Vault("vault encryption failed".into()))?;
        let env = VaultEnvelope {
            version: 1,
            nonce_b64: B64.encode(nonce_bytes),
            ciphertext_b64: B64.encode(ciphertext),
        };
        let encoded = serde_json::to_vec_pretty(&env)?;
        write_owner_only_atomic(&self.vault_path, &encoded)
    }

    pub fn set(&self, name: &str, value: &str) -> Result<()> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| ZenkoError::Vault("vault lock poisoned".into()))?;
        let mut map = self.load_map()?;
        map.insert(name.to_string(), value.to_string());
        self.save_map(&map)
    }

    pub fn get(&self, name: &str) -> Result<Option<Zeroizing<String>>> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| ZenkoError::Vault("vault lock poisoned".into()))?;
        let map = self.load_map()?;
        Ok(map.get(name).cloned().map(Zeroizing::new))
    }

    pub fn remove(&self, name: &str) -> Result<bool> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| ZenkoError::Vault("vault lock poisoned".into()))?;
        let mut map = self.load_map()?;
        let removed = map.remove(name).is_some();
        match removed {
            true => self.save_map(&map)?,
            false => {}
        }
        Ok(removed)
    }

    pub fn has(&self, name: &str) -> Result<bool> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| ZenkoError::Vault("vault lock poisoned".into()))?;
        let map = self.load_map()?;
        Ok(map.contains_key(name))
    }

    /// Returns safe read-only vault metadata (key names, count, file size, cipher).
    /// Strictly NEVER exposes secret values.
    pub fn stats(&self) -> VaultStats {
        let exists = self.vault_path.exists();
        let vault_bytes = std::fs::metadata(&self.vault_path).map(|m| m.len()).unwrap_or(0);
        let key_bytes = std::fs::metadata(&self.key_path).map(|m| m.len()).unwrap_or(0);
        let key_names = match self.write_lock.lock() {
            Ok(_guard) => self
                .load_map()
                .map(|m| m.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        VaultStats {
            exists,
            encrypted: exists && self.key_path.exists(),
            cipher: "ChaCha20-Poly1305".to_string(),
            path: self.vault_path.display().to_string(),
            size_bytes: vault_bytes + key_bytes,
            key_count: key_names.len(),
            key_names,
        }
    }

    /// Returns all currently stored secret values (used strictly in-memory by the log redactor).
    pub fn all_values(&self) -> Vec<String> {
        let Ok(_guard) = self.write_lock.lock() else {
            return Vec::new();
        };
        self.load_map()
            .map(|map| map.into_values().filter(|v| v.len() >= 6).collect())
            .unwrap_or_default()
    }

    pub fn generate_token(bytes: usize) -> String {
        let mut buf = vec![0u8; bytes];
        rand::thread_rng().fill_bytes(&mut buf);
        hex::encode(buf)
    }

    pub fn generate_pairing_code() -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
        let mut out = String::with_capacity(6);
        let mut rng = rand::thread_rng();
        let mut buf = [0u8; 6];
        rng.fill_bytes(&mut buf);
        for b in buf {
            out.push(ALPHABET[(b as usize) % ALPHABET.len()] as char);
        }
        out
    }

    pub fn hash_secret(input: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        hex::encode(hasher.finalize())
    }

    pub fn verify_secret_eq(a: &str, b: &str) -> bool {
        let ha = Sha256::digest(a.as_bytes());
        let hb = Sha256::digest(b.as_bytes());
        ha.as_slice().ct_eq(hb.as_slice()).into()
    }
}

/// Atomically writes `bytes` into a unique sibling temp file created directly with `0o600`
/// permissions (preventing any world-readable TOCTOU window), then renames into `target`.
pub fn write_owner_only_atomic(target: &Path, bytes: &[u8]) -> Result<()> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let file_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("vault");
    let tmp = parent.join(format!(".{file_name}.{}.tmp", crate::new_id()));

    let write_res = (|| -> Result<()> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(FILE_MODE_OWNER_ONLY);
        }
        let mut file = opts.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(err) = write_res {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }

    if let Err(err) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn vault_roundtrip_and_constant_time() {
        let dir = std::env::temp_dir().join(format!("zenko_vault_test_{}", crate::new_id()));
        let vault = Arc::new(SecretVault::open(dir.join("vault.enc"), dir.join("vault.key")));
        vault.set("OPENROUTER_API_KEY", "sk-or-test-123").ok();
        let val = vault.get("OPENROUTER_API_KEY").ok().flatten();
        assert_eq!(val.as_deref().map(String::as_str), Some("sk-or-test-123"));
        assert!(SecretVault::verify_secret_eq("abc", "abc"));
        assert!(!SecretVault::verify_secret_eq("abc", "abd"));

        // Concurrent writes must not drop keys
        let v1 = vault.clone();
        let v2 = vault.clone();
        let h1 = std::thread::spawn(move || v1.set("K1", "V1"));
        let h2 = std::thread::spawn(move || v2.set("K2", "V2"));
        assert!(h1.join().is_ok());
        assert!(h2.join().is_ok());
        assert_eq!(vault.get("K1").ok().flatten().as_deref().map(String::as_str), Some("V1"));
        assert_eq!(vault.get("K2").ok().flatten().as_deref().map(String::as_str), Some("V2"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn read_only_ops_do_not_create_key_when_vault_exists_without_key() {
        let dir = std::env::temp_dir().join(format!("zenko_vault_nokey_{}", crate::new_id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let vault_path = dir.join("vault.enc");
        let key_path = dir.join("vault.key");
        let vault = SecretVault::open(&vault_path, &key_path);
        vault.set("SECRET", "top-secret-value").expect("set");
        std::fs::remove_file(&key_path).expect("remove key");
        assert!(!key_path.exists());

        assert!(vault.get("SECRET").is_err(), "get must fail closed when key is missing");
        assert!(vault.has("SECRET").is_err(), "has must fail closed when key is missing");
        let stats = vault.stats();
        assert!(!stats.encrypted, "stats must not claim encrypted without key");
        assert!(!key_path.exists(), "read-only vault ops must NEVER generate a bogus vault.key");

        let _ = std::fs::remove_dir_all(dir);
    }
}
