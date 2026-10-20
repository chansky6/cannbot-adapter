//! File logging setup.
//!
//! Logs go to `<app_log_dir>/` as daily rolling files
//! (`cannbot-adapter.log.YYYY-MM-DD`, plus a `cannbot-adapter.log`
//! symlink to the current file), in addition to stderr.

use std::path::PathBuf;
use tauri::Manager;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

pub fn log_dir(app: &tauri::AppHandle) -> PathBuf {
    app.path().app_log_dir().expect("app log dir unavailable")
}

/// Install the tracing subscriber: stderr + daily rolling file (fan-out).
pub fn init(app: &tauri::AppHandle) {
    let dir = log_dir(app);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("failed to create log dir {dir:?}: {e}");
    }
    let file_appender = tracing_appender::rolling::daily(&dir, "cannbot-adapter.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    // The guard must live for the whole process; leak it deliberately.
    std::mem::forget(guard);

    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    // stderr is unbuffered; stdout would silently swallow logs when
    // redirected to a file and the process is killed.
    let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);
    let file_layer = tracing_subscriber::fmt::layer().with_writer(file_writer);
    tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer)
        .with(file_layer)
        .init();
}
