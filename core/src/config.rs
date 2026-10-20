use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::secret;
use crate::upstream::ModelsRegistry;

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

/// Cross-field validation rules shared by every frontend. `known_models`
/// is the freshest models registry, if one has been synced yet.
///
/// An alias equal to a real gateway model name would silently hijack
/// requests for that model (aliases are resolved before the registry
/// lookup), so reject the collision instead.
pub fn validate(cfg: &AppConfig, known_models: Option<&ModelsRegistry>) -> Result<(), String> {
    if cfg.bind == "0.0.0.0" && cfg.effective_api_key().is_none() {
        return Err("监听 0.0.0.0（局域网）时必须设置本地 API Key".to_string());
    }
    if cfg.port == 0 {
        return Err("端口无效".to_string());
    }
    if cfg.vk_policy == VkPolicy::Manual && cfg.effective_manual_vks().is_empty() {
        return Err("VK 策略为「手动指定」时必须填写至少一个 VK".to_string());
    }
    if let Some(reg) = known_models {
        let ids = reg.model_ids();
        let real: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
        for (alias, target) in &cfg.model_aliases {
            if alias == target {
                return Err(format!("别名「{alias}」不能指向自身"));
            }
            if real.contains(alias.as_str()) {
                return Err(format!(
                    "别名「{alias}」与真实模型名冲突，会导致该模型无法直接调用，请换一个别名"
                ));
            }
        }
    } else {
        for (alias, target) in &cfg.model_aliases {
            if alias == target {
                return Err(format!("别名「{alias}」不能指向自身"));
            }
        }
    }
    Ok(())
}

pub fn config_file(dir: &Path) -> PathBuf {
    dir.join("config.json")
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
    b.iter()
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join("-")
}

pub fn load_or_init(dir: &Path) -> AppConfig {
    let path = config_file(dir);
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
                        match secret::decrypt(dir, &vk) {
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
                        match secret::decrypt(dir, &mv) {
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
                        tracing::error!(
                            "VK 解密失败（.keyfile 可能丢失或损坏），请重新配置个人 VK"
                        );
                    }
                    // Persist with a fresh random nonce (also migrates legacy
                    // plaintext / zero-nonce values to the current format).
                    if needs_reencrypt {
                        if let Err(e) = save_config(dir, &cfg) {
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
    if let Err(e) = save_config(dir, &cfg) {
        tracing::warn!(error = %e, "failed to persist initial config");
    }
    cfg
}

pub fn save_config(dir: &Path, cfg: &AppConfig) -> Result<(), String> {
    let path = config_file(dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create config dir failed: {e}"))?;
    }
    // Persist an encrypted copy; the in-memory config keeps plaintext.
    let mut persisted = cfg.clone();
    if let Some(vk) = persisted.user_vk.take() {
        persisted.user_vk = Some(secret::encrypt(dir, &vk)?);
    }
    if let Some(mv) = persisted.manual_vk.take() {
        persisted.manual_vk = Some(secret::encrypt(dir, &mv)?);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cfg() -> AppConfig {
        AppConfig {
            port: 8266,
            ..AppConfig::default()
        }
    }

    #[test]
    fn lan_bind_requires_api_key() {
        let mut cfg = test_cfg();
        cfg.bind = "0.0.0.0".into();
        assert!(validate(&cfg, None).is_err());
        cfg.api_key = Some("sk-test".into());
        assert!(validate(&cfg, None).is_ok());
    }

    #[test]
    fn zero_port_rejected() {
        let mut cfg = test_cfg();
        cfg.port = 0;
        assert!(validate(&cfg, None).is_err());
    }

    #[test]
    fn manual_policy_requires_vks() {
        let mut cfg = test_cfg();
        cfg.vk_policy = VkPolicy::Manual;
        assert!(validate(&cfg, None).is_err());
        cfg.manual_vk = Some("vk-1,".into());
        assert!(validate(&cfg, None).is_ok());
    }

    #[test]
    fn alias_self_reference_rejected() {
        let mut cfg = test_cfg();
        cfg.model_aliases.insert("gpt-4".into(), "gpt-4".into());
        assert!(validate(&cfg, None).is_err());
    }

    #[test]
    fn alias_colliding_with_real_model_rejected() {
        let mut cfg = test_cfg();
        cfg.model_aliases.insert("glm-5".into(), "glm-4.7".into());
        let reg = ModelsRegistry {
            entries: vec![crate::upstream::ModelEntry {
                title: "GLM".into(),
                model: "glm-5".into(),
                vk: "vk-x".into(),
                context_length: None,
                max_tokens: None,
                builtin: true,
            }],
            fetched_at: std::time::SystemTime::now(),
        };
        assert!(validate(&cfg, Some(&reg)).is_err());
        // Without a registry the collision can't be detected — must pass.
        assert!(validate(&cfg, None).is_ok());
    }

    #[test]
    fn manual_vks_parsing() {
        let mut cfg = test_cfg();
        cfg.manual_vk = Some(" vk-1 , ,vk-2,".into());
        assert_eq!(cfg.effective_manual_vks(), vec!["vk-1", "vk-2"]);
    }
}
