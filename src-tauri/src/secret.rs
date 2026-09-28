//! At-rest encryption for secrets persisted in config.json.
//!
//! The only secret that matters is the personal VK. It is stored as
//! `enc:v1:<base64(nonce || ciphertext)>` using AES-256-GCM. The key is
//! machine-bound: it is derived (SHA-256) from the app config dir path
//! concatenated with a random salt kept next to config.json (`.keyfile`).
//! This raises the bar for casual leaks of config.json (sync/share/backup)
//! without pretending to defend against a determined local attacker.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use sha2::{Digest, Sha256};

use crate::config::config_dir;

pub const ENC_PREFIX: &str = "enc:v1:";

/// Marker returned to the frontend instead of the plaintext secret.
pub const MASK_SENTINEL: &str = "**********";

fn key_path(app: &tauri::AppHandle) -> std::path::PathBuf {
    config_dir(app).join(".keyfile")
}

/// Load or create the random salt; derive the AES key from it.
fn load_key(app: &tauri::AppHandle) -> Result<[u8; 32], String> {
    let path = key_path(app);
    let salt = if path.exists() {
        let s = std::fs::read(&path).map_err(|e| format!("read keyfile: {e}"))?;
        if s.len() != 32 {
            return Err("keyfile corrupted (wrong length)".to_string());
        }
        s
    } else {
        use rand::RngCore;
        let mut s = [0u8; 32];
        rand::rng().fill_bytes(&mut s);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create config dir: {e}"))?;
        }
        std::fs::write(&path, s).map_err(|e| format!("write keyfile: {e}"))?;
        // 0600 to keep the salt private (same as config.json).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        s.to_vec()
    };
    // Bind the key to this install location as well.
    let dir = config_dir(app);
    let dir_str = dir.to_string_lossy();
    let mut hasher = Sha256::new();
    hasher.update(salt);
    hasher.update(dir_str.as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&hasher.finalize());
    Ok(key)
}

fn b64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn b64_decode(text: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|e| format!("base64 decode: {e}"))
}

/// True when the stored value is already in encrypted form.
pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(ENC_PREFIX)
}

/// Encrypt a plaintext secret into `enc:v1:<base64(nonce || ciphertext)>`.
/// A fresh random 96-bit nonce is generated per encryption and prepended to
/// the ciphertext. Values already carrying the prefix are returned unchanged.
pub fn encrypt(app: &tauri::AppHandle, plain: &str) -> Result<String, String> {
    if is_encrypted(plain) {
        return Ok(plain.to_string());
    }
    use rand::RngCore;
    let key = load_key(app)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    let mut nonce_bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plain.as_bytes())
        .map_err(|e| format!("encrypt: {e}"))?;
    // nonce || ciphertext
    let mut blob = nonce_bytes.to_vec();
    blob.extend_from_slice(&ct);
    Ok(format!("{ENC_PREFIX}{}", b64_encode(&blob)))
}

/// Decrypt an `enc:v1:<base64(nonce || ciphertext)>` value back to plaintext.
/// Plaintext input (legacy configs) is passed through unchanged.
/// Blobs written by the initial buggy build (zero nonce, no nonce prefix)
/// fall back to the zero nonce; lengths alone can't distinguish the formats.
pub fn decrypt(app: &tauri::AppHandle, stored: &str) -> Result<String, String> {
    if !is_encrypted(stored) {
        return Ok(stored.to_string());
    }
    let key = load_key(app)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    let blob = b64_decode(stored.trim_start_matches(ENC_PREFIX))?;
    if blob.len() < 12 {
        return Err("encrypted blob too short".to_string());
    }
    let utf8 = |pt: Vec<u8>| String::from_utf8(pt).map_err(|e| format!("decrypted value not utf-8: {e}"));
    // New format: nonce(12) || ciphertext.
    if blob.len() >= 28 {
        if let Ok(pt) = cipher.decrypt(Nonce::from_slice(&blob[..12]), &blob[12..]) {
            return utf8(pt);
        }
    }
    // Legacy zero-nonce format: ciphertext only.
    cipher
        .decrypt(Nonce::from_slice(&[0u8; 12]), blob.as_slice())
        .map_err(|e| format!("decrypt: {e}"))
        .and_then(utf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_detection() {
        assert!(is_encrypted("enc:v1:AAAA"));
        assert!(!is_encrypted("vk-plain"));
        assert!(!is_encrypted(""));
    }
}
