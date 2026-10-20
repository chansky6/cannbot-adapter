use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::upstream::ModelsRegistry;

pub use crate::jwt::JwtState;

/// Backend-agnostic event notification: `(event name, JSON payload)`.
/// The desktop app forwards events to the WebView (and updates its tray);
/// the CLI logs them. Known events: `server-status`, `jwt-status`,
/// `models-synced`.
pub type EventSink = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

/// Shared application state, used by both the local HTTP server and
/// whichever frontend (desktop app or CLI) drives the core.
pub struct SharedState {
    /// Directory holding `config.json`, `.keyfile` and `usage.db`. Kept so
    /// late consumers (usage accounting, secret re-encryption) resolve the
    /// same paths the config was loaded from.
    pub config_dir: PathBuf,
    pub config: std::sync::RwLock<AppConfig>,
    pub jwt: std::sync::RwLock<JwtState>,
    pub models: std::sync::RwLock<Option<ModelsRegistry>>,
    pub server: std::sync::RwLock<Option<ServerHandle>>,
    pub server_error: std::sync::RwLock<Option<String>>,
    /// Serializes VK authentication so concurrent tasks don't re-authenticate
    /// at the same time (e.g. startup race between setup and models sync).
    pub auth_lock: tokio::sync::Mutex<()>,
    pub http: reqwest::Client,
    /// Handle to the tokio runtime the frontend runs on; used to spawn
    /// background tasks (server, models sync, usage accounting).
    pub runtime: tokio::runtime::Handle,
    /// Event notifier (see [`EventSink`]).
    pub events: EventSink,
}

pub struct ServerHandle {
    pub addr: String,
    pub cancel: CancellationToken,
    pub task: tokio::task::JoinHandle<()>,
}

pub type Shared = Arc<SharedState>;

impl SharedState {
    /// Build the shared state: HTTP client, config (loaded or initialized
    /// under `config_dir`) and empty runtime state.
    pub fn new(
        config_dir: PathBuf,
        runtime: tokio::runtime::Handle,
        events: EventSink,
    ) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| format!("failed to build http client: {e}"))?;
        let config = crate::config::load_or_init(&config_dir);
        Ok(Self {
            config_dir,
            config: std::sync::RwLock::new(config),
            jwt: std::sync::RwLock::new(JwtState::default()),
            models: std::sync::RwLock::new(None),
            server: std::sync::RwLock::new(None),
            server_error: std::sync::RwLock::new(None),
            auth_lock: tokio::sync::Mutex::new(()),
            http,
            runtime,
            events,
        })
    }

    /// The OpenAI-compatible base URL clients should use, derived from the
    /// running server (or the configured port when the server is down).
    pub fn api_base(&self) -> String {
        let server_addr = self
            .server
            .read()
            .expect("server lock poisoned")
            .as_ref()
            .map(|h| h.addr.clone());
        let cfg = self.config.read().expect("config lock poisoned");
        format!(
            "http://{}/v1",
            server_addr
                .as_deref()
                .unwrap_or(&format!("127.0.0.1:{}", cfg.port))
        )
    }
}

/// Notify the frontend about a state change. Serialization of the payload
/// can't realistically fail for our view types; failures are logged and
/// swallowed (an event is always advisory).
pub fn notify(shared: &Shared, event: &str, payload: impl serde::Serialize) {
    match serde_json::to_value(payload) {
        Ok(v) => (shared.events)(event, v),
        Err(e) => tracing::warn!(event, error = %e, "failed to serialize event payload"),
    }
}
