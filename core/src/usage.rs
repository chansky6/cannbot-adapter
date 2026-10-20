//! Local token usage accounting.
//!
//! Every successful `/v1/chat/completions` response is tapped by `proxy.rs`;
//! this module extracts the `usage` field (non-stream JSON, or the final SSE
//! `data:` frame carrying usage) and aggregates it into a small SQLite
//! database: one row per (day, model) with token counts and request count.
//!
//! Recording happens regardless of which frontend drives the core; only the
//! *query* side is frontend-specific (the desktop app shows charts, the CLI
//! offers no query command by design).

use rusqlite::Connection;
use serde::Serialize;
use std::path::PathBuf;

use crate::state::Shared;

#[derive(Debug, Clone, Serialize)]
pub struct UsageRow {
    pub date: String,
    pub model: String,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Parsed usage numbers from a response body.
#[derive(Debug, Clone, Copy, Default)]
struct TokenCounts {
    input: u64,
    output: u64,
}

fn db_path(shared: &Shared) -> PathBuf {
    shared.config_dir.join("usage.db")
}

fn with_conn<T>(shared: &Shared, f: impl FnOnce(&Connection) -> T) -> Result<T, String> {
    let path = db_path(shared);
    let conn = Connection::open(&path).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS usage (
            date TEXT NOT NULL,
            model TEXT NOT NULL,
            requests INTEGER NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (date, model)
        );",
    )
    .map_err(|e| e.to_string())?;
    Ok(f(&conn))
}

/// Parse usage counts from a full response body.
/// `stream` selects the extraction strategy:
/// - non-stream: the JSON `usage` object;
/// - stream: the last `data:` frame containing a `usage` object (many gateways
///   only include it in the final chunk when requested).
fn extract_usage(body: &[u8], stream: bool) -> Option<TokenCounts> {
    if stream {
        extract_usage_sse(body)
    } else {
        extract_usage_json(body)
    }
}

fn extract_usage_json(body: &[u8]) -> Option<TokenCounts> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let u = v.get("usage")?;
    Some(TokenCounts {
        input: u.get("prompt_tokens")?.as_u64()?,
        output: u.get("completion_tokens")?.as_u64()?,
    })
}

fn extract_usage_sse(body: &[u8]) -> Option<TokenCounts> {
    let text = std::str::from_utf8(body).ok()?;
    let mut last: Option<TokenCounts> = None;
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        let Some(u) = v.get("usage").filter(|u| !u.is_null()) else {
            continue;
        };
        let input = u.get("prompt_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
        let output = u
            .get("completion_tokens")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        last = Some(TokenCounts { input, output });
    }
    last
}

/// Record a completed response. Called from a blocking task; failures are
/// logged and swallowed (accounting must never break the proxy).
pub fn record_from_body(shared: &Shared, model: &str, stream: bool, body: &[u8]) {
    let Some(counts) = extract_usage(body, stream) else {
        tracing::debug!(model, "no usage info in response body");
        return;
    };
    if let Err(e) = record(shared, model, counts) {
        tracing::warn!(model, error = %e, "failed to record usage");
    }
}

fn record(shared: &Shared, model: &str, counts: TokenCounts) -> Result<(), String> {
    with_conn(shared, |conn| {
        // Let SQLite compute the local date so recording and querying
        // (`date('now', 'localtime')` in get_usage_rows) can never disagree.
        let date: String = conn
            .query_row("SELECT date('now', 'localtime')", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO usage (date, model, requests, input_tokens, output_tokens)
             VALUES (?1, ?2, 1, ?3, ?4)
             ON CONFLICT(date, model) DO UPDATE SET
                requests = requests + 1,
                input_tokens = input_tokens + ?3,
                output_tokens = output_tokens + ?4",
            rusqlite::params![date, model, counts.input, counts.output],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?
}

/// Aggregate usage rows for the frontend.
pub fn get_usage_rows(shared: &Shared, days: Option<u32>) -> Result<Vec<UsageRow>, String> {
    let limit = days.unwrap_or(30) as i64;
    with_conn(shared, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT date, model, requests, input_tokens, output_tokens
                 FROM usage
                 WHERE date >= date('now', 'localtime', ?1)
                 ORDER BY date DESC, output_tokens DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![format!("-{limit} days")], |r| {
                Ok(UsageRow {
                    date: r.get(0)?,
                    model: r.get(1)?,
                    requests: r.get(2)?,
                    input_tokens: r.get(3)?,
                    output_tokens: r.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    })?
}

/// Remove all usage records.
pub fn clear_usage_rows(shared: &Shared) -> Result<(), String> {
    with_conn(shared, |conn| {
        conn.execute("DELETE FROM usage", [])
            .map_err(|e| e.to_string())?;
        Ok(())
    })?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_usage() {
        let body = br#"{"id":"x","usage":{"prompt_tokens":10,"completion_tokens":20}}"#;
        let c = extract_usage(body, false).unwrap();
        assert_eq!((c.input, c.output), (10, 20));
    }

    #[test]
    fn extracts_sse_last_usage() {
        let body = b"data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":9}}\n\ndata: [DONE]\n";
        let c = extract_usage(body, true).unwrap();
        assert_eq!((c.input, c.output), (7, 9));
    }

    #[test]
    fn no_usage_returns_none() {
        assert!(extract_usage(b"{}", false).is_none());
    }
}
