use std::time::Duration;

use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tauri::Emitter;
use tokio_util::sync::CancellationToken;

use crate::state::{ServerHandle, Shared};
use crate::{jwt, upstream, usage, util};

/// Classifications of upstream failures that drive the VK fallback loop.
pub enum UpstreamError {
    /// 401 from the gateway — JWT rejected, may be worth re-reading the session file.
    Unauthorized(String),
    /// Quota exhausted / rate limited on the current VK — try the next one.
    Quota(String),
    /// Any other upstream error, forwarded to the client as-is.
    Other(u16, String),
    Network(String),
}

/// POST one chat request to the gateway with the given JWT + VK.
/// On HTTP 200 the response is already converted to an axum `Response`
/// (streaming body, both for SSE and plain JSON). The full body is buffered
/// through so usage accounting can read the final token counts.
pub async fn send_chat(
    shared: &Shared,
    jwt: &str,
    vk: &str,
    body: &Value,
) -> Result<Response, UpstreamError> {
    let url = format!(
        "{}/gateway/compatible-mode/v1/chat/completions",
        upstream::GATEWAY_BASE
    );
    let resp = shared
        .http
        .post(&url)
        .bearer_auth(jwt)
        .header("x-api-vkey", vk)
        .json(body)
        .send()
        .await
        .map_err(|e| UpstreamError::Network(e.to_string()))?;
    let status = resp.status();
    if status.is_success() {
        let model = body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let stream = body
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let response = forward_response(shared, resp, model, stream).await;
        Ok(response)
    } else {
        let text = resp.text().await.unwrap_or_default();
        let msg = format!("upstream {status}: {}", truncate(&text, 500));
        if status.as_u16() == 401 {
            Err(UpstreamError::Unauthorized(msg))
        } else if is_quota_error(status.as_u16(), &text) {
            Err(UpstreamError::Quota(msg))
        } else {
            Err(UpstreamError::Other(status.as_u16(), msg))
        }
    }
}

fn is_quota_error(status: u16, body: &str) -> bool {
    let lower = body.to_lowercase();
    status == 429
        || ((status == 403 || status == 402)
            && (lower.contains("quota") || body.contains("配额")))
}

/// Max bytes buffered from a response body for usage extraction. The final
/// SSE chunk (with usage) or the JSON `usage` field sits at the very end of
/// the body, so a generous tail budget is enough; anything beyond is dropped
/// and the request simply isn't accounted.
const USAGE_TAP_LIMIT: usize = 256 * 1024;

/// Convert a successful `reqwest` response into a streaming axum response.
/// The body passes through a tap so the final usage (token counts) can be
/// recorded once the stream completes, without delaying the client.
async fn forward_response(
    shared: &Shared,
    upstream: reqwest::Response,
    model: String,
    stream: bool,
) -> Response {
    let status = StatusCode::from_u16(upstream.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let mut builder = Response::builder().status(status).header("x-adapter", "cannbot");
    if let Some(ct) = content_type {
        builder = builder.header("content-type", ct);
    }

    // Shared tap buffer; capped so huge responses can't balloon memory.
    // Keeps the TAIL of the body: the usage field (JSON `usage` object, or
    // the final SSE frame carrying it) sits at the very end, so once the
    // limit is reached older bytes are dropped from the front.
    let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let tap_buf = buf.clone();
    let tapped = upstream.bytes_stream().inspect(move |item| {
        if let Ok(bytes) = item {
            let mut b = tap_buf.lock().expect("usage tap lock poisoned");
            b.extend_from_slice(bytes);
            if b.len() > USAGE_TAP_LIMIT {
                let excess = b.len() - USAGE_TAP_LIMIT;
                b.drain(..excess);
            }
        }
    });

    // When the body has been fully forwarded, run usage extraction on the
    // buffered copy. The sentinel item yields nothing to the client (the
    // unfold's `None` terminates the stream) and accounting happens in a
    // blocking task so the HTTP task is never blocked on SQLite.
    let done_buf = buf;
    let done_shared = shared.clone();
    let done_model = model;
    let done_stream = stream;
    let with_tail = tapped.chain(futures_util::stream::once(async move {
        let body = done_buf.lock().expect("usage tap lock poisoned").clone();
        let shared = done_shared.clone();
        let model = done_model.clone();
        tauri::async_runtime::spawn_blocking(move || {
            usage::record_from_body(&shared, &model, done_stream, &body);
        });
        // Match the upstream stream's item type (reqwest::Error).
        Ok::<bytes::Bytes, reqwest::Error>(bytes::Bytes::new())
    }));

    builder
        .body(Body::from_stream(with_tail))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn openai_error(status: StatusCode, msg: impl Into<String>) -> Response {
    let body = json!({
        "error": {"message": msg.into(), "type": "adapter_error", "code": status.as_u16()}
    });
    (status, Json(body)).into_response()
}

fn truncate(s: &str, max: usize) -> String {
    util::truncate(s, max)
}

async fn chat_completions(State(shared): State<Shared>, axum::extract::Json(body): axum::extract::Json<Value>) -> Response {
    let Some(requested_model) = body.get("model").and_then(Value::as_str).map(str::to_string) else {
        return openai_error(StatusCode::BAD_REQUEST, "请求体缺少 `model` 字段");
    };

    // alias -> real model
    let real_model = {
        let cfg = shared.config.read().expect("config lock poisoned");
        cfg.model_aliases
            .get(&requested_model)
            .cloned()
            .unwrap_or(requested_model.clone())
    };
    let mut body = body;
    if let Some(obj) = body.as_object_mut() {
        obj.insert("model".into(), Value::String(real_model.clone()));
    }

    // VK candidates according to the configured policy
    let vks = vk_candidates(&shared, &real_model).await;
    if vks.is_empty() {
        let has_vk = shared
            .config
            .read()
            .expect("config lock poisoned")
            .effective_user_vk()
            .is_some();
        return openai_error(
            StatusCode::SERVICE_UNAVAILABLE,
            if has_vk {
                "没有可用的虚拟密钥（VK）：请等待模型列表同步，或在配置页检查 VK 策略/手动 VK 设置"
            } else {
                "尚未配置个人 VK：请在配置页「上游凭据」中填写从公司获取的个人 VK"
            },
        );
    }

    let mut auth_retried = false;
    let mut idx = 0;
    while idx < vks.len() {
        let jwt_token = match upstream::current_jwt(&shared).await {
            Some(t) => t,
            None => match jwt::authenticate(&shared).await.token {
                Some(t) => t,
                None => {
                    return openai_error(
                        StatusCode::UNAUTHORIZED,
                        "JWT 未就绪：请检查配置页的 VK 设置",
                    )
                }
            },
        };
        let vk = vks[idx].clone();
        match send_chat(&shared, &jwt_token, &vk, &body).await {
            Ok(resp) => return resp,
            Err(UpstreamError::Unauthorized(msg)) => {
                if auth_retried {
                    return openai_error(
                        StatusCode::UNAUTHORIZED,
                        format!("上游持续拒绝 JWT（{msg}）。请检查 VK 是否有效。"),
                    );
                }
                auth_retried = true;
                // Re-authenticate (VK -> fresh JWT), then retry with the same VK.
                if jwt::authenticate(&shared).await.token.is_none() {
                    return openai_error(
                        StatusCode::UNAUTHORIZED,
                        "VK 重新认证失败，请检查配置页的 VK 设置",
                    );
                }
                tracing::info!("re-authenticated after upstream 401, retrying same VK");
                continue; // retry same VK with the fresh JWT
            }
            Err(UpstreamError::Quota(msg)) => {
                tracing::warn!(vk = vk_prefix(&vk), "VK quota issue, falling back to next: {msg}");
                idx += 1;
                continue;
            }
            Err(UpstreamError::Other(status, msg)) => {
                let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
                return openai_error(code, msg);
            }
            Err(UpstreamError::Network(e)) => {
                return openai_error(StatusCode::BAD_GATEWAY, format!("上游网络错误: {e}"));
            }
        }
    }
    openai_error(
        StatusCode::TOO_MANY_REQUESTS,
        "所有虚拟密钥均配额不足或不可用，请明日再试或检查账号配额",
    )
}

fn vk_prefix(vk: &str) -> String {
    vk.chars().take(10).collect()
}

/// Pick ordered VK candidates for a request, according to the configured policy.
async fn vk_candidates(shared: &Shared, real_model: &str) -> Vec<String> {
    use crate::config::VkPolicy;
    let cfg = shared.config.read().expect("config lock poisoned").clone();
    let registry = shared.models.read().expect("models lock poisoned").clone();
    match cfg.vk_policy {
        VkPolicy::Auto => match registry {
            Some(r) => {
                let v = r.vks_for(real_model);
                if v.is_empty() {
                    tracing::warn!(model = %real_model, "model not in registry, trying all known VKs");
                    r.all_vks()
                } else {
                    v
                }
            }
            None => Vec::new(),
        },
        VkPolicy::BuiltinOnly => match registry.and_then(|r| r.builtin_vk()) {
            Some(vk) => vec![vk],
            None => {
                tracing::warn!("builtin VK requested but not available yet");
                Vec::new()
            }
        },
        VkPolicy::PersonalOnly => match registry.and_then(|r| r.personal_vk()) {
            Some(vk) => vec![vk],
            None => {
                tracing::warn!("personal VK requested but not available yet");
                Vec::new()
            }
        },
        VkPolicy::Manual => {
            let vks = cfg.effective_manual_vks();
            if vks.is_empty() {
                tracing::warn!("manual VK policy set but manual_vk is empty");
            }
            vks
        }
    }
}

async fn list_models(State(shared): State<Shared>) -> Response {
    let registry = shared.models.read().expect("models lock poisoned").clone();
    let Some(reg) = registry else {
        return openai_error(StatusCode::SERVICE_UNAVAILABLE, "模型列表尚未同步完成，请稍后重试");
    };
    let mut ids = reg.model_ids();
    let aliases: Vec<String> = {
        let cfg = shared.config.read().expect("config lock poisoned");
        cfg.model_aliases.keys().cloned().collect()
    };
    for alias in aliases {
        if !ids.contains(&alias) {
            ids.push(alias);
        }
    }
    let data: Vec<Value> = ids
        .into_iter()
        .map(|id| {
            json!({"id": id, "object": "model", "created": 0, "owned_by": "cannbot"})
        })
        .collect();
    Json(json!({"object": "list", "data": data})).into_response()
}

async fn health() -> Response {
    Json(json!({"status": "ok", "service": "cannbot-adapter"})).into_response()
}

async fn fallback_404() -> Response {
    openai_error(
        StatusCode::NOT_FOUND,
        "未知端点；可用端点: POST /v1/chat/completions, GET /v1/models, GET /health",
    )
}

async fn auth_mw(State(shared): State<Shared>, req: Request, next: Next) -> Response {
    // CORS preflight (OPTIONS) must pass without auth — browsers never
    // attach the API key to preflight requests.
    if req.method() == axum::http::Method::OPTIONS {
        return next.run(req).await;
    }
    if req.uri().path() == "/health" {
        return next.run(req).await;
    }
    let expected = shared
        .config
        .read()
        .expect("config lock poisoned")
        .effective_api_key();
    if let Some(key) = expected {
        let provided = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| {
                v.strip_prefix("Bearer ")
                    .or_else(|| v.strip_prefix("bearer "))
                    .map(str::to_string)
            })
            .or_else(|| {
                req.headers()
                    .get("x-api-key")
                    .or_else(|| req.headers().get("api-key"))
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            });
        match provided {
            Some(p) if p.trim() == key => {}
            _ => {
                return openai_error(
                    StatusCode::UNAUTHORIZED,
                    "无效的 API Key，请携带 Authorization: Bearer <key>",
                )
            }
        }
    }
    next.run(req).await
}

async fn access_log(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let start = std::time::Instant::now();
    let resp = next.run(req).await;
    tracing::info!(
        %method,
        %path,
        status = resp.status().as_u16(),
        elapsed_ms = start.elapsed().as_millis() as u64,
        "request"
    );
    resp
}

fn build_router(shared: Shared) -> Router {
    // The config UI (WebView origin) probes models via fetch; allow only the
    // known WebView / dev origins, never arbitrary websites.
    // NOTE: cors must be added LAST so it wraps the auth layer — otherwise
    // 401 responses from auth would bypass CORS headers entirely.
    let allowed: Vec<axum::http::HeaderValue> = [
        "tauri://localhost",
        "http://tauri.localhost",
        "http://localhost:1420",
        "http://127.0.0.1:1420",
    ]
    .iter()
    .filter_map(|v| v.parse().ok())
    .collect();
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::AllowOrigin::list(allowed))
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers(tower_http::cors::Any);
    Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/models", get(list_models))
        .route("/health", get(health))
        .fallback(fallback_404)
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024))
        .layer(middleware::from_fn(access_log))
        .layer(middleware::from_fn_with_state(shared.clone(), auth_mw))
        .layer(cors)
        .with_state(shared)
}

pub async fn start_server(shared: Shared) -> Result<(), String> {
    stop_server(&shared).await;
    let cfg = shared.config.read().expect("config lock poisoned").clone();
    if cfg.bind == "0.0.0.0" && cfg.effective_api_key().is_none() {
        let msg = "监听 0.0.0.0（局域网）时必须设置本地 API Key".to_string();
        *shared.server_error.write().expect("server error lock poisoned") = Some(msg.clone());
        return Err(msg);
    }
    let bind = if cfg.bind == "0.0.0.0" { "0.0.0.0" } else { "127.0.0.1" };
    let addr = format!("{bind}:{}", cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| {
            let msg = format!("绑定 {addr} 失败（端口可能被占用）: {e}");
            *shared.server_error.write().expect("server error lock poisoned") = Some(msg.clone());
            msg
        })?;
    let local = listener
        .local_addr()
        .map_err(|e| format!("获取监听地址失败: {e}"))?;
    let app = build_router(shared.clone());
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let task = tauri::async_runtime::spawn(async move {
        let serve =
            axum::serve(listener, app).with_graceful_shutdown(async move { token.cancelled().await });
        if let Err(e) = serve.await {
            tracing::error!(error = %e, "http server stopped with error");
        }
    });
    *shared.server.write().expect("server lock poisoned") = Some(ServerHandle {
        addr: local.to_string(),
        cancel,
        task,
    });
    *shared.server_error.write().expect("server error lock poisoned") = None;
    tracing::info!(%local, "http server started");
    let _ = shared.app.emit(
        "server-status",
        json!({"running": true, "addr": local.to_string(), "error": null}),
    );
    Ok(())
}

pub async fn stop_server(shared: &Shared) {
    let stopped = shared
        .server
        .write()
        .expect("server lock poisoned")
        .take();
    if let Some(handle) = stopped {
        handle.cancel.cancel();
        // Give graceful shutdown a short window, then force-abort
        // (long-lived SSE streams would otherwise block the port).
        tokio::time::sleep(Duration::from_millis(200)).await;
        handle.task.abort();
        tracing::info!("http server stopped");
        let _ = shared.app.emit(
            "server-status",
            json!({"running": false, "addr": null, "error": null}),
        );
    }
}
