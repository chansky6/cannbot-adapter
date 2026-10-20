//! UI-independent core of the CANNBot adapter.
//!
//! Everything the adapter actually does lives here: loading and encrypting
//! the config, exchanging the user's VK for a JWT, syncing the model list,
//! and running the local OpenAI-compatible HTTP proxy. Both frontends —
//! the Tauri desktop app and the `cannbotcli` CLI — are thin shells over
//! this crate and must not implement business logic themselves.
//!
//! The core is deliberately free of any GUI toolkit dependency so the CLI
//! builds and runs on headless Linux servers.

pub mod config;
pub mod jwt;
pub mod proxy;
pub mod secret;
pub mod state;
pub mod upstream;
pub mod usage;
pub mod util;

pub use config::AppConfig;
pub use state::{EventSink, Shared, SharedState};
