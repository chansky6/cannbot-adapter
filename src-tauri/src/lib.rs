mod config;
mod jwt;
mod proxy;
mod state;
mod tray;
mod upstream;

use std::sync::Arc;
use std::time::UNIX_EPOCH;

use serde::Serialize;
use serde_json::json;
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::MacosLauncher;

use crate::config::AppConfig;
use crate::jwt::JwtStatusView;
use crate::proxy::UpstreamError;
use crate::state::{JwtState, Shared, SharedState};

#[derive(Serialize, Clone)]
pub struct VkStatusView {
    /// Masked builtin VK, e.g. "vk-e9f099…ce5b", if fetched from the gateway.
    pub builtin: Option<String>,
    /// Masked personal VK, if fetched.
    pub personal: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct StatusView {
    pub server_running: bool,
    pub server_addr: Option<String>,
    pub server_error: Option<String>,
    pub jwt: JwtStatusView,
    pub vk: VkStatusView,
    pub vk_policy: crate::config::VkPolicy,
    pub models_count: Option<usize>,
    pub models_synced_at: Option<u64>,
    pub alias_count: usize,
    pub api_base: String,
}

fn mask_vk(vk: Option<String>) -> Option<String> {
    vk.map(|v| {
        let head: String = v.chars().take(8).collect();
        let tail: String = v.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
        format!("{head}…{tail}")
    })
}

#[tauri::command]
fn get_config(state: tauri::State<Shared>) -> AppConfig {
    state.config.read().expect("config lock poisoned").clone()
}

#[tauri::command]
async fn save_config(
    state: tauri::State<'_, Shared>,
    new_config: AppConfig,
) -> Result<serde_json::Value, String> {
    if new_config.bind == "0.0.0.0" && new_config.effective_api_key().is_none() {
        return Err("监听 0.0.0.0（局域网）时必须设置本地 API Key".to_string());
    }
    if new_config.port == 0 {
        return Err("端口无效".to_string());
    }
    if new_config.vk_policy == crate::config::VkPolicy::Manual
        && new_config.effective_manual_vks().is_empty()
    {
        return Err("VK 策略为「手动指定」时必须填写至少一个 VK".to_string());
    }
    let needs_restart = {
        let old = state.config.read().expect("config lock poisoned");
        old.port != new_config.port || old.bind != new_config.bind
        // api_key is read per-request, no restart needed
    };
    *state.config.write().expect("config lock poisoned") = new_config.clone();
    config::save_config(&state.app, &new_config)?;

    // user VK may have changed — re-authenticate in the background
    let auth_shared = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        jwt::authenticate(&auth_shared).await;
    });

    let restarted = if needs_restart {
        let shared = state.inner().clone();
        proxy::start_server(shared).await?;
        true
    } else {
        false
    };
    Ok(json!({ "restarted": restarted }))
}

#[tauri::command]
async fn regenerate_api_key(state: tauri::State<'_, Shared>) -> Result<String, String> {
    let key = config::generate_api_key();
    let mut cfg = state.config.read().expect("config lock poisoned").clone();
    cfg.api_key = Some(key.clone());
    *state.config.write().expect("config lock poisoned") = cfg.clone();
    config::save_config(&state.app, &cfg)?;
    Ok(key)
}

#[tauri::command]
fn get_status(state: tauri::State<Shared>) -> StatusView {
    let jwt = state.jwt.read().expect("jwt lock poisoned").clone();
    let models = state.models.read().expect("models lock poisoned").clone();
    let server = state.server.read().expect("server lock poisoned");
    let server_running = server.is_some();
    let server_addr = server.as_ref().map(|h| h.addr.clone());
    let server_error = state
        .server_error
        .read()
        .expect("server error lock poisoned")
        .clone();
    let cfg = state.config.read().expect("config lock poisoned");
    let api_base = format!(
        "http://{}/v1",
        server_addr
            .as_deref()
            .unwrap_or(&format!("127.0.0.1:{}", cfg.port))
    );
    StatusView {
        server_running,
        server_addr,
        server_error,
        jwt: JwtStatusView::from(&jwt),
        vk: VkStatusView {
            builtin: mask_vk(models.as_ref().and_then(|m| m.builtin_vk())),
            personal: mask_vk(models.as_ref().and_then(|m| m.personal_vk())),
        },
        vk_policy: cfg.vk_policy,
        models_count: models.as_ref().map(|m| m.entries.len()),
        models_synced_at: models.as_ref().map(|m| {
            m.fetched_at
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        }),
        alias_count: cfg.model_aliases.len(),
        api_base,
    }
}

#[tauri::command]
fn get_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let autolaunch = app.autolaunch();
    if enabled {
        autolaunch.enable().map_err(|e| e.to_string())?;
    } else {
        autolaunch.disable().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A model entry for the frontend list: real gateway models merged with aliases.
#[derive(Serialize, Clone)]
pub struct ModelInfo {
    pub model: String,
    /// Set when this row is a local alias (`alias_of` = real model).
    pub alias_of: Option<String>,
    /// Supported by the builtin VK (3000w/day pool).
    pub builtin: bool,
    /// Supported by the personal VK (1e8/day pool).
    pub personal: bool,
    pub context_length: Option<u64>,
    pub max_tokens: Option<u64>,
}

#[tauri::command]
fn get_models(state: tauri::State<'_, Shared>) -> Vec<ModelInfo> {
    let registry = state.models.read().expect("models lock poisoned").clone();
    let cfg = state.config.read().expect("config lock poisoned").clone();
    let mut out: Vec<ModelInfo> = Vec::new();
    if let Some(r) = registry {
        for e in &r.entries {
            if let Some(existing) = out.iter_mut().find(|m| m.model == e.model) {
                if e.builtin {
                    existing.builtin = true;
                } else {
                    existing.personal = true;
                }
            } else {
                out.push(ModelInfo {
                    model: e.model.clone(),
                    alias_of: None,
                    builtin: e.builtin,
                    personal: !e.builtin,
                    context_length: e.context_length,
                    max_tokens: e.max_tokens,
                });
            }
        }
    }
    for (alias, target) in &cfg.model_aliases {
        if !out.iter().any(|m| m.model == *alias) {
            out.push(ModelInfo {
                model: alias.clone(),
                alias_of: Some(target.clone()),
                builtin: false,
                personal: false,
                context_length: None,
                max_tokens: None,
            });
        }
    }
    out
}

/// Connectivity probe: JWT -> model list -> one mini chat request (max_tokens=5).
#[tauri::command]
async fn test_connection(state: tauri::State<'_, Shared>) -> Result<serde_json::Value, String> {
    let shared = state.inner().clone();
    let jwt_ok = upstream::current_jwt(&shared).await.is_some();

    let reg = upstream::fetch_models(&shared).await?;
    let model = reg
        .entries
        .iter()
        .find(|e| e.model.contains("flash"))
        .map(|e| e.model.clone())
        .or_else(|| reg.entries.first().map(|e| e.model.clone()))
        .ok_or_else(|| "模型列表为空".to_string())?;
    let vks = {
        let v = reg.vks_for(&model);
        if v.is_empty() {
            reg.all_vks()
        } else {
            v
        }
    };

    let body = json!({
        "model": model,
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 5
    });
    let mut chat_ok = false;
    let mut chat_msg = String::new();
    for vk in &vks {
        let Some(jwt) = upstream::current_jwt(&shared).await else {
            break;
        };
        match proxy::send_chat(&shared, &jwt, vk, &body).await {
            Ok(resp) => {
                chat_ok = true;
                chat_msg = format!("模型 {model} 响应 {}", resp.status());
                break;
            }
            Err(UpstreamError::Quota(m)) => {
                chat_msg = m;
                continue;
            }
            Err(e) => {
                chat_msg = e.message();
                break;
            }
        }
    }

    Ok(json!({
        "jwt_ok": jwt_ok,
        "models_count": reg.entries.len(),
        "chat_ok": chat_ok,
        "chat_model": model,
        "chat_message": chat_msg,
    }))
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        // stderr is unbuffered; stdout would silently swallow logs when
        // redirected to a file and the process is killed.
        .with_writer(std::io::stderr)
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();
            let shared: Shared = Arc::new(SharedState {
                http: reqwest::Client::builder()
                    .connect_timeout(std::time::Duration::from_secs(15))
                    .build()
                    .expect("failed to build http client"),
                app: handle.clone(),
                config: std::sync::RwLock::new(config::load_or_init(&handle)),
                jwt: std::sync::RwLock::new(JwtState::default()),
                models: std::sync::RwLock::new(None),
                server: std::sync::RwLock::new(None),
                server_error: std::sync::RwLock::new(None),
                auth_lock: tokio::sync::Mutex::new(()),
            });
            app.manage(shared.clone());

            let s = shared.clone();
            tauri::async_runtime::spawn(async move {
                jwt::authenticate(&s).await;
            });
            upstream::spawn_sync_loop(shared.clone());

            let s = shared.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = proxy::start_server(s.clone()).await {
                    tracing::error!(error = %e, "http server failed to start");
                    let _ = s.app.emit(
                        "server-status",
                        json!({"running": false, "addr": null, "error": e}),
                    );
                }
            });

            tray::setup(&handle)?;

            if let Some(win) = app.get_webview_window("main") {
                let h = handle.clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        // close-to-tray instead of quitting
                        api.prevent_close();
                        if let Some(w) = h.get_webview_window("main") {
                            let _ = w.hide();
                        }
                    }
                });
            }

            tray::show_main_window(&handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            regenerate_api_key,
            get_status,
            get_models,
            get_autostart,
            set_autostart,
            test_connection
        ])
        .run(tauri::generate_context!())
        .expect("error while running cannbot-adapter");
}
