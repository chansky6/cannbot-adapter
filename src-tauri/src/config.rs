use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::secret;

/// How the adapter picks the upstream virtual key (VK) for a request.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VkPolicy {
    /// Auto fallback: builtin VK first (auto-discovered via the model list),
    /// switch to the user VK on quota errors.
    #[default]
    Auto,
    /// Only the builtin VK (3000w tokens/day, 3 flash models only).
    BuiltinOnly,
    /// Only the user-provided VK (1e8/day + 1e9/month, full allowlist).
    PersonalOnly,
    /// Only the VK(s) manually provided in `manual_vk`.
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub port: u16,
    /// "127.0.0.1" (loopback only) or "0.0.0.0" (LAN)
    pub bind: String,
    /// Local API key. `None` / empty disables authentication.
    #[serde(default)]
    pub api_key: Option<String>,
    /// The user's personal VK, obtained from the company. This is the only
    /// upstream credential the user needs to provide; the JWT is derived
    /// from it via `/api/auth/authenticate`.
    ///
    /// On disk this is stored AES-256-GCM encrypted (`enc:v1:…`, see
    /// `secret.rs`). After `load_or_init` the in-memory value is plaintext.
    #[serde(default)]
    pub user_vk: Option<String>,
    /// Stable random device identifier (MAC-format) used for VK authentication.
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub vk_policy: VkPolicy,
    /// Manually provided VK(s), comma separated. Only used when
    /// `vk_policy` is `Manual`.
    #[serde(default)]
    pub manual_vk: Option<String>,
    /// alias -> real gateway model name
    #[serde(default)]
    pub model_aliases: HashMap<String, String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            port: 8266,
            bind: "127.0.0.1".to_string(),
            api_key: None,
            user_vk: None,
            device_id: None,
            vk_policy: VkPolicy::default(),
            manual_vk: None,
            model_aliases: HashMap::new(),
        }
    }
}

impl AppConfig {
    pub fn effective_api_key(&self) -> Option<String> {
        self.api_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }

    pub fn effective_user_vk(&self) -> Option<String> {
        self.user_vk
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }

    /// Manual VK list (trimmed, non-empty, comma separated).
    pub fn effective_manual_vks(&self) -> Vec<String> {
        self.manual_vk
            .as_deref()
            .map(|s| {
                s.split(',')
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The stable device id used in `/api/auth/authenticate`; generated on first use.
    pub fn device_id_or_init(&mut self) -> String {
        if let Some(id) = self
            .device_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return id.to_string();
        }
        let id = generate_device_id();
        self.device_id = Some(id.clone());
        id
    }
}

pub fn config_dir(app: &tauri::AppHandle) -> PathBuf {
    use tauri::Manager;
    app.path()
        .app_config_dir()
        .expect("app config dir unavailable")
}

pub fn config_file(app: &tauri::AppHandle) -> PathBuf {
    config_dir(app).join("config.json")
}

pub fn generate_api_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 24];
    rand::rng().fill_bytes(&mut bytes);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("sk-{hex}")
}

/// Random MAC-format device identifier, e.g. "a3-5f-01-9c-22-b4".
/// Random (instead of the real hardware MAC) for privacy; persisted so the
/// gateway sees a stable "device" across restarts.
fn generate_device_id() -> String {
    use rand::RngCore;
    let mut b = [0u8; 6];
    rand::rng().fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join("-")
}

pub fn load_or_init(app: &tauri::AppHandle) -> AppConfig {
    let path = config_file(app);
    if path.exists() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            match serde_json::from_str::<AppConfig>(&text) {
                Ok(mut cfg) => {
                    // Decrypt secrets into memory; legacy plaintext values
                    // pass through unchanged (they get encrypted on next save).
                    // A decrypt failure keeps the stored ciphertext so the
                    // value isn't silently lost — the user is told to re-enter
                    // it, and the frontend shows an auth error anyway.
                    let mut needs_reencrypt = false;
                    let mut decrypt_failed = false;
                    if let Some(vk) = cfg.user_vk.take() {
                        match secret::decrypt(app, &vk) {
                            Ok(plain) => {
                                // Re-save whenever the stored form wasn't the
                                // current nonce-prefixed format (plaintext or
                                // legacy zero-nonce ciphertext): save_config
                                // re-encrypts with a fresh random nonce.
                                needs_reencrypt |= true;
                                cfg.user_vk = Some(plain);
                            }
                            Err(e) => {
                                decrypt_failed = true;
                                tracing::error!(error = %e, "failed to decrypt user_vk; keep ciphertext, user must re-enter it");
                                cfg.user_vk = Some(vk);
                            }
                        }
                    }
                    if let Some(mv) = cfg.manual_vk.take() {
                        match secret::decrypt(app, &mv) {
                            Ok(plain) => {
                                needs_reencrypt |= true;
                                cfg.manual_vk = Some(plain);
                            }
                            Err(e) => {
                                decrypt_failed = true;
                                tracing::error!(error = %e, "failed to decrypt manual_vk; keep ciphertext, user must re-enter it");
                                cfg.manual_vk = Some(mv);
                            }
                        }
                    }
                    tracing::info!(?path, "config loaded");
                    if decrypt_failed {
                        tracing::error!("VK 解密失败（.keyfile 可能丢失或损坏），请重新配置个人 VK");
                    }
                    // Persist with a fresh random nonce (also migrates legacy
                    // plaintext / zero-nonce values to the current format).
                    if needs_reencrypt {
                        if let Err(e) = save_config(app, &cfg) {
                            tracing::warn!(error = %e, "failed to re-encrypt secrets");
                        }
                    }
                    return cfg;
                }
                Err(e) => {
                    tracing::warn!(?path, error = %e, "config parse failed, using defaults");
                }
            }
        }
    }
    let cfg = AppConfig {
        api_key: Some(generate_api_key()),
        ..AppConfig::default()
    };
    if let Err(e) = save_config(app, &cfg) {
        tracing::warn!(error = %e, "failed to persist initial config");
    }
    cfg
}

pub fn save_config(app: &tauri::AppHandle, cfg: &AppConfig) -> Result<(), String> {
    let path = config_file(app);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create config dir failed: {e}"))?;
    }
    // Persist an encrypted copy; the in-memory config keeps plaintext.
    let mut persisted = cfg.clone();
    if let Some(vk) = persisted.user_vk.take() {
        persisted.user_vk = Some(secret::encrypt(app, &vk)?);
    }
    if let Some(mv) = persisted.manual_vk.take() {
        persisted.manual_vk = Some(secret::encrypt(app, &mv)?);
    }
    let text =
        serde_json::to_string_pretty(&persisted).map_err(|e| format!("serialize config: {e}"))?;
    // Atomic write: a crash mid-write must never leave a truncated config.json
    // (it holds the encrypted VK; losing it forces the user to re-enter it).
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("write config failed: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("replace config failed: {e}"))?;
    tracing::info!(?path, "config saved");
    Ok(())
}
