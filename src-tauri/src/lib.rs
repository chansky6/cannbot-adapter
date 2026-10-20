//! Tauri desktop app — a thin shell over `cannbot-core`.
//!
//! This crate owns everything GUI-specific (windows, tray, autostart, file
//! logging, Tauri commands and event forwarding); all business logic lives
//! in the core crate shared with the `cannbotcli` CLI.

mod logging;
mod tray;

use std::sync::Arc;

use serde::Serialize;
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::MacosLauncher;

use cannbot_core::config::{self, AppConfig};
use cannbot_core::jwt::{self, JwtStatusView};
use cannbot_core::proxy;
use cannbot_core::state::{EventSink, Shared, SharedState};
use cannbot_core::upstream;
use cannbot_core::usage;

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
    pub vk_policy: config::VkPolicy,
    pub models_count: Option<usize>,
    pub models_synced_at: Option<u64>,
    pub alias_count: usize,
    pub api_base: String,
}

/// Fully mask an auto-discovered VK for display: the gateway VKs are also
/// secrets, so never show even a prefix/suffix.
fn mask_vk(vk: Option<String>) -> Option<String> {
    vk.map(|_| "**********".to_string())
}

#[tauri::command]
fn get_config(state: tauri::State<Shared>) -> AppConfig {
    let mut cfg = state.config.read().expect("config lock poisoned").clone();
    // Never send the plaintext VK to the frontend; a fully masked value means
    // "a VK is configured" and the backend keeps the real secret.
    cfg.user_vk = if cfg.user_vk.is_some() {
        Some(cannbot_core::secret::MASK_SENTINEL.to_string())
    } else {
        None
    };
    // manual_vk may hold real secrets too — replace with the sentinel when a
    // list is configured; the frontend round-trips it to mean "unchanged".
    if cfg.manual_vk.is_some() {
        cfg.manual_vk = Some(cannbot_core::secret::MASK_SENTINEL.to_string());
    }
    cfg
}

#[tauri::command]
async fn save_config(
    state: tauri::State<'_, Shared>,
    new_config: AppConfig,
) -> Result<serde_json::Value, String> {
    // Merge the incoming config with stored secrets: the frontend round-trips
    // masked sentinel values, so a sentinel means "unchanged".
    let mut incoming = new_config;
    {
        let old = state.config.read().expect("config lock poisoned");
        if incoming.user_vk.as_deref() == Some(cannbot_core::secret::MASK_SENTINEL) {
            incoming.user_vk = old.user_vk.clone();
        }
        if incoming.manual_vk.as_deref() == Some(cannbot_core::secret::MASK_SENTINEL) {
            incoming.manual_vk = old.manual_vk.clone();
        }
    }
    {
        // Alias collision checks need the freshest models registry.
        let models = state.models.read().expect("models lock poisoned");
        config::validate(&incoming, models.as_ref())?;
    }
    let needs_restart = {
        let old = state.config.read().expect("config lock poisoned");
        old.port != incoming.port || old.bind != incoming.bind
        // api_key is read per-request, no restart needed
    };
    *state.config.write().expect("config lock poisoned") = incoming.clone();
    config::save_config(&state.config_dir, &incoming)?;

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
    Ok(serde_json::json!({ "restarted": restarted }))
}

#[tauri::command]
async fn regenerate_api_key(state: tauri::State<'_, Shared>) -> Result<String, String> {
    let key = config::generate_api_key();
    let mut cfg = state.config.read().expect("config lock poisoned").clone();
    cfg.api_key = Some(key.clone());
    *state.config.write().expect("config lock poisoned") = cfg.clone();
    config::save_config(&state.config_dir, &cfg)?;
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
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        }),
        alias_count: cfg.model_aliases.len(),
        api_base: state.api_base(),
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
    /// Alias configured for this real model, if any (edit-in-place field).
    pub alias: Option<String>,
    /// Supported by the builtin VK (3000w/day pool).
    pub builtin: bool,
    /// Supported by the personal VK (1e8/day pool).
    pub personal: bool,
    pub context_length: Option<u64>,
    pub max_tokens: Option<u64>,
}

#[tauri::command]
fn get_models(state: tauri::State<Shared>) -> Vec<ModelInfo> {
    let registry = state.models.read().expect("models lock poisoned").clone();
    let cfg = state.config.read().expect("config lock poisoned").clone();
    let mut out: Vec<ModelInfo> = Vec::new();
    if let Some(r) = registry {
        for e in &r.entries {
            let alias = cfg
                .model_aliases
                .iter()
                .find(|(_, target)| *target == &e.model)
                .map(|(a, _)| a.clone());
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
                    alias,
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
                alias: None,
                builtin: false,
                personal: false,
                context_length: None,
                max_tokens: None,
            });
        }
    }
    out
}

/// Fetch the model list from the gateway and refresh the registry.
/// No chat request, no quota use.
#[tauri::command]
async fn fetch_models_cmd(state: tauri::State<'_, Shared>) -> Result<serde_json::Value, String> {
    let shared = state.inner().clone();
    let reg = upstream::fetch_models(&shared).await?;
    Ok(serde_json::json!({ "models_count": reg.entries.len() }))
}

#[tauri::command]
fn get_usage(
    state: tauri::State<Shared>,
    days: Option<u32>,
) -> Result<Vec<usage::UsageRow>, String> {
    usage::get_usage_rows(&state, days)
}

#[tauri::command]
fn clear_usage(state: tauri::State<Shared>) -> Result<(), String> {
    usage::clear_usage_rows(&state)
}

pub fn run() {
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

            logging::init(app.handle());
            let handle = app.handle().clone();

            // Core wiring: same config dir the CLI uses, tauri's async
            // runtime for background tasks, and an event sink that forwards
            // core events to the WebView and keeps the tray tooltip fresh.
            let config_dir = app
                .path()
                .app_config_dir()
                .expect("app config dir unavailable");
            let runtime = tauri::async_runtime::handle().inner().clone();
            let ev_handle = handle.clone();
            let events: EventSink = Arc::new(move |event, payload| {
                let _ = ev_handle.emit(event, payload.clone());
                if event == "jwt-status" {
                    if let Ok(view) = serde_json::from_value::<JwtStatusView>(payload) {
                        tray::set_status_tooltip(&ev_handle, &view);
                    }
                }
            });
            let shared: Shared = Arc::new(
                SharedState::new(config_dir, runtime, events).expect("failed to init shared state"),
            );
            app.manage(shared.clone());

            let s = shared.clone();
            tauri::async_runtime::spawn(async move {
                jwt::authenticate(&s).await;
            });
            upstream::spawn_sync_loop(shared.clone());

            let s = shared.clone();
            let err_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = proxy::start_server(s.clone()).await {
                    tracing::error!(error = %e, "http server failed to start");
                    let _ = err_handle.emit(
                        "server-status",
                        serde_json::json!({"running": false, "addr": null, "error": e}),
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
            fetch_models_cmd,
            get_usage,
            clear_usage
        ])
        .run(tauri::generate_context!())
        .expect("error while running cannbot-adapter");
}
