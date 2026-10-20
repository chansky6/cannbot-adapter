use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::state::{self, Shared};

pub const GATEWAY_BASE: &str = "https://cannbot.hicann.cn";

#[derive(Debug, Clone, Serialize)]
pub struct ModelEntry {
    pub title: String,
    /// Real gateway model name (the `model` field for requests)
    pub model: String,
    /// Virtual key bound to this entry
    pub vk: String,
    pub context_length: Option<u64>,
    pub max_tokens: Option<u64>,
    pub builtin: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelsRegistry {
    /// builtin entries first, personal entries after
    pub entries: Vec<ModelEntry>,
    pub fetched_at: std::time::SystemTime,
}

impl ModelsRegistry {
    /// Ordered VK candidates for a model: builtin VK first, then personal.
    pub fn vks_for(&self, model: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for e in &self.entries {
            if e.model == model && !out.contains(&e.vk) {
                out.push(e.vk.clone());
            }
        }
        out
    }

    /// All known VKs (builtin first) — used when the model is unknown to the registry.
    pub fn all_vks(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for e in &self.entries {
            if !out.contains(&e.vk) {
                out.push(e.vk.clone());
            }
        }
        out
    }

    pub fn builtin_vk(&self) -> Option<String> {
        self.entries
            .iter()
            .find(|e| e.builtin)
            .map(|e| e.vk.clone())
    }

    pub fn personal_vk(&self) -> Option<String> {
        self.entries
            .iter()
            .find(|e| !e.builtin)
            .map(|e| e.vk.clone())
    }

    /// Deduplicated real model names.
    pub fn model_ids(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for e in &self.entries {
            if !out.contains(&e.model) {
                out.push(e.model.clone());
            }
        }
        out
    }
}

#[derive(Deserialize)]
struct ModelsApiResp {
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    data: Option<ModelsApiData>,
}

#[derive(Deserialize, Default)]
struct ModelsApiData {
    #[serde(default, rename = "builtinModels")]
    builtin_models: Vec<ModelsApiItem>,
    #[serde(default, rename = "personalModels")]
    personal_models: Vec<ModelsApiItem>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ModelsApiItem {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    completion_options: Option<CompletionOptions>,
}

#[derive(Deserialize, Clone, Default)]
struct CompletionOptions {
    #[serde(default)]
    max_tokens: Option<u64>,
}

pub async fn current_jwt(shared: &Shared) -> Option<String> {
    shared.jwt.read().expect("jwt lock poisoned").token.clone()
}

/// Fetch the model list from the gateway and build the registry.
/// Returns `Err` with a marker prefix on HTTP 401 so callers can react to an
/// expired/revoked JWT (invalidate + re-authenticate).
pub async fn fetch_models(shared: &Shared) -> Result<ModelsRegistry, String> {
    const AUTH_FAILED: &str = "auth-failed:";
    let jwt = current_jwt(shared)
        .await
        .ok_or(format!("{AUTH_FAILED} JWT 未就绪（VK 未配置或认证失败）"))?;
    let url = format!("{GATEWAY_BASE}/cannbot/api/models/personal");
    let resp = shared
        .http
        .get(&url)
        .bearer_auth(&jwt)
        .send()
        .await
        .map_err(|e| format!("请求模型列表失败: {e}"))?;
    let status = resp.status();
    if status.as_u16() == 401 {
        return Err(format!("{AUTH_FAILED} 模型列表接口返回 401：JWT 已失效"));
    }
    let text = resp
        .text()
        .await
        .map_err(|e| format!("读取模型列表响应失败: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "模型列表接口返回 {status}: {}",
            crate::util::truncate(&text, 300)
        ));
    }
    let parsed: ModelsApiResp =
        serde_json::from_str(&text).map_err(|e| format!("解析模型列表失败: {e}"))?;
    if parsed.success == Some(false) {
        return Err("模型列表接口返回 success=false".to_string());
    }
    let data = parsed.data.unwrap_or_default();
    let mut entries: Vec<ModelEntry> = Vec::new();
    for (items, builtin) in [(data.builtin_models, true), (data.personal_models, false)] {
        for item in items {
            let (Some(model), Some(vk)) = (item.model.clone(), item.api_key.clone()) else {
                continue;
            };
            if model.trim().is_empty() || vk.trim().is_empty() {
                continue;
            }
            entries.push(ModelEntry {
                title: item.title.clone().unwrap_or_else(|| model.clone()),
                model,
                vk,
                context_length: item.context_length,
                max_tokens: item.completion_options.as_ref().and_then(|c| c.max_tokens),
                builtin,
            });
        }
    }
    if entries.is_empty() {
        return Err("模型列表为空".to_string());
    }
    let count = entries.len();
    let registry = ModelsRegistry {
        entries,
        fetched_at: std::time::SystemTime::now(),
    };
    *shared.models.write().expect("models lock poisoned") = Some(registry.clone());
    tracing::info!(count, "models registry refreshed");
    state::notify(
        shared,
        "models-synced",
        serde_json::json!({ "count": count }),
    );
    Ok(registry)
}

/// Background loop: keep the models registry fresh.
/// Ensures a JWT exists (VK authentication) before each fetch attempt;
/// retry every 30s until the first success, then refresh every 10 minutes.
/// A 401 from the models endpoint invalidates the cached JWT so the next
/// iteration re-authenticates (the short retry delay keeps recovery fast).
pub fn spawn_sync_loop(shared: Shared) {
    const AUTH_FAILED: &str = "auth-failed:";
    let runtime = shared.runtime.clone();
    runtime.spawn(async move {
        loop {
            if current_jwt(&shared).await.is_none() {
                let _ = crate::jwt::authenticate(&shared).await;
            }
            let mut fetch_failed = false;
            if let Err(e) = fetch_models(&shared).await {
                fetch_failed = true;
                if let Some(reason) = e.strip_prefix(AUTH_FAILED) {
                    tracing::warn!(
                        reason,
                        "models fetch rejected our JWT; invalidating and re-authenticating"
                    );
                    crate::jwt::invalidate(&shared);
                } else {
                    tracing::warn!(error = %e, "models fetch failed, will retry");
                }
            }
            let delay = if shared
                .models
                .read()
                .expect("models lock poisoned")
                .is_some()
            {
                Duration::from_secs(600)
            } else {
                Duration::from_secs(30)
            };
            // Re-auth after an invalidation should not wait a full cycle.
            let delay = if fetch_failed && current_jwt(&shared).await.is_none() {
                Duration::from_secs(3)
            } else {
                delay
            };
            tokio::time::sleep(delay).await;
        }
    });
}
