use serde::{Deserialize, Serialize};

use crate::config;
use crate::state::{self, Shared};

pub const AUTH_URL: &str = "https://cannbot.hicann.cn/cannbot/api/auth/authenticate";

/// Internal JWT state. `token` must never be serialized to a frontend in
/// full — views should go through [`JwtStatusView`].
#[derive(Debug, Clone, Default)]
pub struct JwtState {
    pub token: Option<String>,
    pub message: String,
}

/// Drop the cached token (e.g. after an upstream 401 on a non-chat endpoint)
/// so the next `authenticate()` actually re-authenticates instead of
/// short-circuiting on the stale token.
pub fn invalidate(shared: &Shared) {
    let mut jwt = shared.jwt.write().expect("jwt lock poisoned");
    if jwt.token.is_some() {
        jwt.token = None;
        jwt.message = "JWT 已失效，等待重新认证".to_string();
        tracing::info!("cached JWT invalidated");
    }
}

/// Frontend-safe view of the authentication state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtStatusView {
    pub ok: bool,
    pub message: String,
}

impl From<&JwtState> for JwtStatusView {
    fn from(s: &JwtState) -> Self {
        Self {
            ok: s.token.is_some(),
            message: s.message.clone(),
        }
    }
}

#[derive(Deserialize)]
struct AuthResp {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(default, rename = "expiresIn")]
    expires_in: Option<i64>,
}

/// Exchange the user-provided VK for a long-lived JWT via `/api/auth/authenticate`.
/// This is the ONLY way the adapter obtains a JWT — no opencode dependency.
pub async fn authenticate(shared: &Shared) -> JwtState {
    // Serialize concurrent authentication attempts; after acquiring the lock,
    // short-circuit if another task has already refreshed the token.
    let _guard = shared.auth_lock.lock().await;
    {
        let existing = shared.jwt.read().expect("jwt lock poisoned");
        if existing.token.is_some() {
            return existing.clone();
        }
    }

    // (vk, mac, need_save)
    let (vk, mac, need_save) = {
        let mut cfg = shared.config.write().expect("config lock poisoned");
        let old = cfg.device_id.clone();
        let mac = cfg.device_id_or_init();
        let need_save = cfg.device_id != old;
        (cfg.effective_user_vk(), mac, need_save)
    };
    if need_save {
        let cfg = shared.config.read().expect("config lock poisoned").clone();
        let _ = config::save_config(&shared.config_dir, &cfg);
    }

    let state = match vk {
        None => JwtState {
            token: None,
            message: "尚未配置个人 VK：请先设置个人 VK".to_string(),
        },
        Some(vk) => {
            let resp = shared
                .http
                .post(AUTH_URL)
                .header("x-api-vkey", &vk)
                .json(&serde_json::json!({"type": "vk", "mac": mac}))
                .send()
                .await;
            match resp {
                Ok(r) if r.status().as_u16() == 200 => match r.json::<AuthResp>().await {
                    Ok(a) => {
                        let days = a.expires_in.map(|s| s / 86400).unwrap_or(0);
                        tracing::info!(days, "vk authenticated, jwt issued");
                        JwtState {
                            token: Some(a.access_token),
                            message: "VK 认证成功".to_string(),
                        }
                    }
                    Err(e) => JwtState {
                        token: None,
                        message: format!("VK 认证响应解析失败: {e}"),
                    },
                },
                Ok(r) => {
                    let code = r.status().as_u16();
                    let body = r.text().await.unwrap_or_default();
                    JwtState {
                        token: None,
                        message: format!(
                            "VK 认证失败（{code}）: {}",
                            crate::util::truncate(&body, 200)
                        ),
                    }
                }
                Err(e) => JwtState {
                    token: None,
                    message: format!("VK 认证网络错误: {e}"),
                },
            }
        }
    };

    let view = JwtStatusView::from(&state);
    let ok = view.ok;
    *shared.jwt.write().expect("jwt lock poisoned") = state;
    tracing::info!(ok, message = %view.message, "jwt state updated");
    // The desktop app uses this event to refresh the UI and update its tray
    // tooltip; the CLI just logs it.
    state::notify(shared, "jwt-status", &view);
    shared.jwt.read().expect("jwt lock poisoned").clone()
}
