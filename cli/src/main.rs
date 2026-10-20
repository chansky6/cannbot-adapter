//! cannbotcli — the CLI frontend of the CANNBot adapter.
//!
//! A thin shell over `cannbot-core`: it loads the same config directory as
//! the desktop app, exposes configuration commands and runs the local
//! OpenAI-compatible proxy in the foreground. It deliberately has no usage
//! query command (the desktop app owns that view).
//!
//! Default config dir matches the Tauri desktop app, e.g. on Linux:
//! `~/.config/cn.hicann.cannbot-adapter`.

use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cannbot_core as core;
use cannbot_core::config::{AppConfig, VkPolicy};
use cannbot_core::state::{EventSink, Shared, SharedState};
use clap::{Parser, Subcommand};

/// Tauri bundle identifier of the desktop app; its config dir is shared.
const IDENTIFIER: &str = "cn.hicann.cannbot-adapter";

#[derive(Parser)]
#[command(
    name = "cannbotcli",
    version,
    about = "CANNBot 网关的本地 OpenAI 兼容适配器（CLI）",
    long_about = "CANNBot 网关的本地 OpenAI 兼容适配器（CLI）。\n\
                  与桌面版共用配置目录；在无图形界面的 Linux 服务器上配置并运行本地代理。"
)]
struct Cli {
    /// 配置目录（默认与桌面版共用；Linux 为 ~/.config/cn.hicann.cannbot-adapter）
    #[arg(long, global = true, value_name = "DIR")]
    config_dir: Option<PathBuf>,

    /// 提高日志详细度：-v 显示 info，-vv 显示 debug，-vvv 显示 trace
    #[arg(short, long, action = clap::ArgAction::Count, global = true, default_value_t = 0)]
    verbose: u8,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 启动本地代理服务（前台运行，Ctrl-C / SIGTERM 优雅退出）
    Serve,
    /// 检查 VK 认证与模型列表同步状态
    Status,
    /// 拉取并列出可用模型与本地别名
    Models,
    /// 查看与修改配置
    #[command(subcommand)]
    Config(ConfigCmd),
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// 打印当前配置（含敏感字段明文，请自行注意终端留存）
    Show,
    /// 设置配置项：port | bind | api-key | vk | vk-policy | manual-vk
    ///
    /// 值为 "-" 或省略时从 stdin 读取（可避免敏感值进入 shell 历史）。
    /// 设为空字符串表示清除该字段。
    Set { key: String, value: Option<String> },
    /// 重新生成本地 API Key 并打印
    RegenerateApiKey,
    /// 管理模型别名
    #[command(subcommand)]
    Alias(AliasCmd),
}

#[derive(Subcommand)]
enum AliasCmd {
    /// 新增或更新别名：alias -> 真实模型名
    Add { alias: String, model: String },
    /// 删除别名
    Remove { alias: String },
    /// 列出全部别名
    List,
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("错误: {e}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let dir = resolve_config_dir(cli.config_dir)?;
    match cli.command {
        Cmd::Serve => serve(dir, cli.verbose),
        Cmd::Status => status(dir, cli.verbose),
        Cmd::Models => models(dir, cli.verbose),
        Cmd::Config(cmd) => config_cmd(dir, cmd),
    }
}

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

fn resolve_config_dir(flag: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(dir) = flag {
        return Ok(dir);
    }
    let base = dirs::config_dir().ok_or_else(|| {
        "无法确定用户配置目录（$HOME 未设置？），请用 --config-dir 指定".to_string()
    })?;
    Ok(base.join(IDENTIFIER))
}

fn init_tracing(verbose: u8) {
    use tracing_subscriber::EnvFilter;
    let filter = match verbose {
        0 => EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        1 => "info".into(),
        2 => "debug".into(),
        _ => "trace".into(),
    };
    // Logs go to stderr so command output on stdout stays pipeable.
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

/// CLI event sink: surface core state changes as log lines.
fn cli_event_sink() -> EventSink {
    Arc::new(|event: &str, payload: serde_json::Value| match event {
        "jwt-status" => {
            let ok = payload.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let msg = payload
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if ok {
                tracing::info!("VK 认证: {msg}");
            } else {
                tracing::warn!("VK 认证: {msg}");
            }
        }
        "server-status" => match payload.get("running").and_then(|v| v.as_bool()) {
            Some(true) => tracing::info!(
                addr = ?payload.get("addr"),
                "server-status: running"
            ),
            _ => tracing::warn!(error = ?payload.get("error"), "server-status: stopped"),
        },
        "models-synced" => {
            tracing::info!(count = ?payload.get("count"), "模型列表已同步");
        }
        _ => {}
    })
}

/// Build a runtime + SharedState and drive `f` to completion. Used by the
/// commands that need the full core (serve / status / models).
fn with_shared<F>(dir: PathBuf, verbose: u8, f: F) -> Result<(), String>
where
    F: for<'a> FnOnce(
        &'a Shared,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), String>> + 'a>,
    >,
{
    init_tracing(verbose);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("创建 tokio runtime 失败: {e}"))?;
    rt.block_on(async move {
        let shared: Shared = Arc::new(SharedState::new(
            dir,
            tokio::runtime::Handle::current(),
            cli_event_sink(),
        )?);
        f(&shared).await
    })
}

// ---------------------------------------------------------------------------
// serve
// ---------------------------------------------------------------------------

fn serve(dir: PathBuf, verbose: u8) -> Result<(), String> {
    with_shared(dir, verbose, move |shared| {
        Box::pin(async move {
            // Fast feedback on a bad VK before the server starts serving
            // requests that are doomed to fail.
            let auth = core::jwt::authenticate(shared).await;
            if auth.token.is_none() {
                eprintln!("警告: {}", auth.message);
                eprintln!(
                    "提示: 先运行 `cannbotcli config set vk <个人VK>` 完成配置；服务仍将启动。"
                );
            }
            core::upstream::spawn_sync_loop(shared.clone());
            core::proxy::start_server(shared.clone()).await?;

            let cfg = shared.config.read().expect("config lock poisoned").clone();
            println!("cannbot 代理服务已启动（前台运行，Ctrl-C 停止）");
            println!("  API Base:   {}", shared.api_base());
            match cfg.effective_api_key() {
                Some(key) => println!("  本地 API Key: {key}"),
                None => println!("  本地 API Key: 未设置（不鉴权）"),
            }
            if cfg.bind == "0.0.0.0" {
                println!("  监听:       0.0.0.0:{}（局域网可访问）", cfg.port);
            }

            wait_shutdown().await;
            println!("收到退出信号，正在停止…");
            core::proxy::stop_server(shared).await;
            println!("已退出");
            Ok(())
        })
    })
}

/// Resolve SIGINT / SIGTERM into a single "shut down" event.
async fn wait_shutdown() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = term.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

// ---------------------------------------------------------------------------
// status / models
// ---------------------------------------------------------------------------

fn status(dir: PathBuf, verbose: u8) -> Result<(), String> {
    with_shared(dir, verbose, |shared| {
        Box::pin(async move {
            let auth = core::jwt::authenticate(shared).await;
            let models = core::upstream::fetch_models(shared).await;
            let cfg = shared.config.read().expect("config lock poisoned").clone();

            println!("配置目录:    {}", shared.config_dir.display());
            match cfg.effective_user_vk() {
                Some(_) => println!("个人 VK:     已配置"),
                None => println!("个人 VK:     未配置"),
            }
            println!("VK 认证:     {}", auth.message);
            match &models {
                Ok(reg) => {
                    let builtin = reg.entries.iter().filter(|e| e.builtin).count();
                    let personal = reg.entries.len() - builtin;
                    println!(
                        "模型列表:    {} 个（内置 {builtin} / 个人 {personal}）",
                        reg.entries.len()
                    );
                }
                Err(e) => println!("模型列表:    同步失败: {e}"),
            }
            println!(
                "本地 API Key: {}",
                if cfg.effective_api_key().is_some() {
                    "已设置"
                } else {
                    "未设置（不鉴权）"
                }
            );
            println!("别名:        {} 个", cfg.model_aliases.len());
            println!(
                "API Base:    http://{}:{}/v1（serve 启动后生效）",
                if cfg.bind == "0.0.0.0" {
                    "0.0.0.0"
                } else {
                    "127.0.0.1"
                },
                cfg.port
            );
            Ok(())
        })
    })
}

fn models(dir: PathBuf, verbose: u8) -> Result<(), String> {
    with_shared(dir, verbose, |shared| {
        Box::pin(async move {
            let auth = core::jwt::authenticate(shared).await;
            if auth.token.is_none() {
                return Err(format!("无法拉取模型列表：{}", auth.message));
            }
            let reg = core::upstream::fetch_models(shared).await?;
            let cfg = shared.config.read().expect("config lock poisoned").clone();

            println!(
                "{:<32} {:<16} {:<6} {:>10} {:>10}",
                "模型", "别名", "来源", "上下文", "最大输出"
            );
            for entry in &reg.entries {
                // alias configured for this real model, if any
                let alias = cfg
                    .model_aliases
                    .iter()
                    .find(|(_, target)| *target == &entry.model)
                    .map(|(a, _)| a.as_str())
                    .unwrap_or("-");
                println!(
                    "{:<32} {:<16} {:<6} {:>10} {:>10}",
                    entry.model,
                    alias,
                    if entry.builtin { "内置" } else { "个人" },
                    entry
                        .context_length
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".into()),
                    entry
                        .max_tokens
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".into()),
                );
            }
            // Aliases pointing at unknown models (typo or stale) — still
            // routable through the proxy, so list them separately.
            let known: std::collections::HashSet<&str> =
                reg.entries.iter().map(|e| e.model.as_str()).collect();
            let orphans: Vec<(&String, &String)> = cfg
                .model_aliases
                .iter()
                .filter(|(_, target)| !known.contains(target.as_str()))
                .collect();
            if !orphans.is_empty() {
                println!();
                println!("指向未知模型的别名（代理仍会按原名转发）:");
                for (alias, target) in orphans {
                    println!("  {alias} -> {target}");
                }
            }
            Ok(())
        })
    })
}

// ---------------------------------------------------------------------------
// config
// ---------------------------------------------------------------------------

fn config_cmd(dir: PathBuf, cmd: ConfigCmd) -> Result<(), String> {
    match cmd {
        ConfigCmd::Show => config_show(&dir),
        ConfigCmd::Set { key, value } => config_set(&dir, &key, value),
        ConfigCmd::RegenerateApiKey => regenerate_api_key(&dir),
        ConfigCmd::Alias(cmd) => alias_cmd(&dir, cmd),
    }
}

fn config_show(dir: &Path) -> Result<(), String> {
    let cfg = core::config::load_or_init(dir);
    println!("配置目录: {}", dir.display());
    println!("配置文件: {}", core::config::config_file(dir).display());
    println!();
    println!("port:       {}", cfg.port);
    println!("bind:       {}", cfg.bind);
    println!(
        "api-key:    {}",
        cfg.effective_api_key()
            .unwrap_or_else(|| "(未设置，不鉴权)".into())
    );
    println!(
        "vk:         {}",
        cfg.effective_user_vk().unwrap_or_else(|| "(未设置)".into())
    );
    println!("vk-policy:  {}", vk_policy_name(&cfg.vk_policy));
    println!(
        "manual-vk:  {}",
        cfg.manual_vk
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("(未设置)")
    );
    println!(
        "device-id:  {}",
        cfg.device_id
            .as_deref()
            .unwrap_or("(未生成，首次认证时生成)")
    );
    println!("model-aliases:");
    if cfg.model_aliases.is_empty() {
        println!("  (无)");
    } else {
        let mut aliases: Vec<(&String, &String)> = cfg.model_aliases.iter().collect();
        aliases.sort();
        for (alias, target) in aliases {
            println!("  {alias} -> {target}");
        }
    }
    println!();
    println!(
        "API Base:   http://{}:{}/v1（serve 启动后生效）",
        if cfg.bind == "0.0.0.0" {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        },
        cfg.port
    );
    Ok(())
}

fn config_set(dir: &Path, key: &str, value: Option<String>) -> Result<(), String> {
    let mut cfg = core::config::load_or_init(dir);
    let raw = match value {
        Some(v) if v != "-" => v,
        _ => read_stdin(stdin_label(key))?,
    };
    let needs_restart = apply_set(&mut cfg, key, &raw)?;
    core::config::validate(&cfg, None)?;
    core::config::save_config(dir, &cfg)?;
    println!("已保存: {key}");
    if needs_restart {
        println!("提示: 端口/监听地址已变更，正在运行的 `cannbotcli serve` 需重启后生效。");
    }
    Ok(())
}

/// Apply one `config set` mutation. Returns whether a running server would
/// need a restart for the change to take effect.
fn apply_set(cfg: &mut AppConfig, key: &str, value: &str) -> Result<bool, String> {
    let non_empty = |v: &str| {
        if v.trim().is_empty() {
            None
        } else {
            Some(v.trim().to_string())
        }
    };
    match key {
        "port" => {
            let port: u16 = value
                .parse()
                .map_err(|_| format!("端口无效: {value}（需 1-65535 的数字）"))?;
            cfg.port = port;
        }
        "bind" => match value.trim() {
            "127.0.0.1" => cfg.bind = "127.0.0.1".into(),
            "0.0.0.0" => cfg.bind = "0.0.0.0".into(),
            other => {
                return Err(format!("bind 仅支持 127.0.0.1 或 0.0.0.0，收到: {other}"));
            }
        },
        "api-key" => cfg.api_key = non_empty(value),
        "vk" | "user-vk" => cfg.user_vk = non_empty(value),
        "vk-policy" => cfg.vk_policy = parse_vk_policy(value)?,
        "manual-vk" => cfg.manual_vk = non_empty(value),
        other => {
            return Err(format!(
                "未知配置项: {other}（可选: port, bind, api-key, vk, vk-policy, manual-vk）"
            ));
        }
    }
    Ok(matches!(key, "port" | "bind"))
}

fn stdin_label(key: &str) -> &'static str {
    match key {
        "vk" | "user-vk" => "个人 VK",
        "api-key" => "本地 API Key",
        "manual-vk" => "手动 VK 列表（逗号分隔）",
        _ => "配置值",
    }
}

fn read_stdin(label: &str) -> Result<String, String> {
    if std::io::stdin().is_terminal() {
        eprintln!("请输入{label}（回车后按 Ctrl-D 结束）:");
    }
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| format!("读取 stdin 失败: {e}"))?;
    Ok(buf.trim().to_string())
}

fn parse_vk_policy(value: &str) -> Result<VkPolicy, String> {
    match value.trim().to_lowercase().as_str() {
        "auto" => Ok(VkPolicy::Auto),
        "builtin" | "builtin_only" => Ok(VkPolicy::BuiltinOnly),
        "personal" | "personal_only" => Ok(VkPolicy::PersonalOnly),
        "manual" => Ok(VkPolicy::Manual),
        other => Err(format!(
            "vk-policy 无效: {other}（可选: auto, builtin, personal, manual）"
        )),
    }
}

fn vk_policy_name(policy: &VkPolicy) -> &'static str {
    match policy {
        VkPolicy::Auto => "auto（内置优先，配额不足回退个人）",
        VkPolicy::BuiltinOnly => "builtin（仅内置 VK）",
        VkPolicy::PersonalOnly => "personal（仅个人 VK）",
        VkPolicy::Manual => "manual（仅手动 VK）",
    }
}

fn regenerate_api_key(dir: &Path) -> Result<(), String> {
    let mut cfg = core::config::load_or_init(dir);
    let key = core::config::generate_api_key();
    cfg.api_key = Some(key.clone());
    core::config::validate(&cfg, None)?;
    core::config::save_config(dir, &cfg)?;
    println!("已重新生成本地 API Key: {key}");
    Ok(())
}

fn alias_cmd(dir: &Path, cmd: AliasCmd) -> Result<(), String> {
    let mut cfg = core::config::load_or_init(dir);
    match cmd {
        AliasCmd::Add { alias, model } => {
            if alias == model {
                return Err(format!("别名「{alias}」不能指向自身"));
            }
            let existed = cfg
                .model_aliases
                .insert(alias.clone(), model.clone())
                .is_some();
            core::config::validate(&cfg, None)?;
            core::config::save_config(dir, &cfg)?;
            if existed {
                println!("已更新别名: {alias} -> {model}");
            } else {
                println!("已添加别名: {alias} -> {model}");
            }
        }
        AliasCmd::Remove { alias } => {
            if cfg.model_aliases.remove(&alias).is_none() {
                return Err(format!("别名不存在: {alias}"));
            }
            core::config::save_config(dir, &cfg)?;
            println!("已删除别名: {alias}");
        }
        AliasCmd::List => {
            if cfg.model_aliases.is_empty() {
                println!("(无别名)");
            } else {
                let mut aliases: Vec<(&String, &String)> = cfg.model_aliases.iter().collect();
                aliases.sort();
                for (alias, target) in aliases {
                    println!("{alias} -> {target}");
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_set_keys() {
        let mut cfg = AppConfig::default();
        assert!(apply_set(&mut cfg, "port", "9000").unwrap());
        assert_eq!(cfg.port, 9000);
        assert!(apply_set(&mut cfg, "bind", "0.0.0.0").unwrap());
        assert_eq!(cfg.bind, "0.0.0.0");
        assert!(!apply_set(&mut cfg, "vk", " vk-abc ").unwrap());
        assert_eq!(cfg.user_vk.as_deref(), Some("vk-abc"));
        apply_set(&mut cfg, "vk", "").unwrap();
        assert_eq!(cfg.user_vk, None);
        assert!(apply_set(&mut cfg, "nope", "x").is_err());
        assert!(apply_set(&mut cfg, "port", "abc").is_err());
        assert!(apply_set(&mut cfg, "bind", "localhost").is_err());
    }

    #[test]
    fn vk_policy_parsing() {
        assert!(matches!(parse_vk_policy("auto").unwrap(), VkPolicy::Auto));
        assert!(matches!(
            parse_vk_policy("Builtin").unwrap(),
            VkPolicy::BuiltinOnly
        ));
        assert!(matches!(
            parse_vk_policy("personal_only").unwrap(),
            VkPolicy::PersonalOnly
        ));
        assert!(matches!(
            parse_vk_policy("manual").unwrap(),
            VkPolicy::Manual
        ));
        assert!(parse_vk_policy("yolo").is_err());
    }
}
