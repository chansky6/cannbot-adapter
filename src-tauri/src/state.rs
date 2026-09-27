use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::upstream::ModelsRegistry;

/// Shared application state, managed by Tauri and used by both the
/// local HTTP server and the config UI.
pub use crate::jwt::JwtState;

pub struct SharedState {
    pub app: tauri::AppHandle,
    pub config: std::sync::RwLock<AppConfig>,
    pub jwt: std::sync::RwLock<JwtState>,
    pub models: std::sync::RwLock<Option<ModelsRegistry>>,
    pub server: std::sync::RwLock<Option<ServerHandle>>,
    pub server_error: std::sync::RwLock<Option<String>>,
    /// Serializes VK authentication so concurrent tasks don't re-authenticate
    /// at the same time (e.g. startup race between setup and models sync).
    pub auth_lock: tokio::sync::Mutex<()>,
    pub http: reqwest::Client,
}

pub struct ServerHandle {
    pub addr: String,
    pub cancel: CancellationToken,
    pub task: tauri::async_runtime::JoinHandle<()>,
}

pub type Shared = Arc<SharedState>;
