# cannbot-adapter

轻量级跨平台（macOS / Windows / Linux CLI）适配器：把 CANNBot 网关（`https://cannbot.hicann.cn/gateway/compatible-mode/v1`）适配为本地 **OpenAI 兼容 API**，任何支持 OpenAI 格式的客户端（ChatBox、IDE 插件、LangChain 等）都能直接接入。

桌面版基于 Tauri 2（Rust + Svelte 5），提供系统托盘 + 配置页；Linux 服务器（无图形界面）可使用 `cannbotcli` 命令行版，与桌面版共用同一套核心与配置。

## 功能

| 功能 | 说明 |
|---|---|
| `/v1/chat/completions` | OpenAI 格式转发，支持 SSE 流式与非流式 |
| `/v1/models` | 从网关拉取模型列表（去重后本地合成，10 分钟自动刷新） |
| 可用模型面板 | 配置页展示全部模型 + VK 通道徽标（双通道/仅内置/仅个人）+ 上下文长度，单行复制 / 一键复制全部；每行可直接配置模型别名 |
| 本地鉴权 | 自动生成 `sk-` API Key，可重新生成；留空关闭鉴权；监听 `0.0.0.0` 时强制要求 |
| VK 自动认证 | 用户只需提供个人 VK（唯一上游凭据），适配器通过 `/api/auth/authenticate` 自动换取 JWT（约 100 年长效），401 时自动重新认证；**无需安装 opencode** |
| VK 加密存储 | 个人 VK / 手动 VK 以 AES-256-GCM 加密落盘（`enc:v1:` 前缀），密钥由本机随机盐派生；前端仅显示 `**********`，明文永不回传；旧明文配置首次启动自动迁移加密 |
| VK 策略可配 | 自动获取网关下发的双 VK；策略可选：自动 fallback（内置 3000 万/日优先，配额不足切个人 1 亿/日）/ 仅内置 / 仅个人 / 手动指定（支持多个按序尝试） |
| 模型别名 | 支持 `gpt-4o → glm-5.3-flash` 之类映射，可在模型列表行内直接配置；兼容写死 OpenAI 模型名的客户端；未命中别名原样透传 |
| Usage 用量页签 | 本地记账（解析响应 `usage` 字段），按日期/模型聚合展示 token 消耗，支持 7/30/90 天范围切换与清空 |
| 日志 | 每日滚动写入 `app_log_dir`（macOS: `~/Library/Logs/cn.hicann.cannbot-adapter/`），配置页「日志」按钮一键打开日志文件夹 |
| 系统集成 | 系统托盘（状态/退出/关闭到托盘）、单实例、开机自启动 |

## 快速开始

### 从安装包运行

从 [Releases](../../releases) 下载：

- macOS: `cannbot-adapter_<version>_aarch64.dmg`（Apple Silicon）
- Windows: `cannbot-adapter_<version>_x64_en-US.msi` 或 `_setup.exe`

> **macOS 安装提示**：应用未经 Apple 开发者签名公证，从浏览器下载后首次打开可能提示「"cannbot-adapter"已损坏，无法打开」。这是 Gatekeeper 对未签名应用的拦截，并非文件真的损坏。安装后执行一次：
>
> ```bash
> xattr -cr /Applications/cannbot-adapter.app
> ```
>
> 即可正常打开（右键 → 打开也可绕过一次）。

启动后，在配置页「上游凭据」中填入**个人 VK**（从公司/管理员处获取，`vk-` 开头）即完成接入，无需安装 opencode。认证流程：适配器用 VK 换取长效 JWT（约 100 年有效），随后自动发现账号下的内置 VK，auto 策略即可用。

### 客户端接入

配置页「接入信息」中复制 API Base 与 API Key，按 OpenAI 方式调用：

```bash
curl http://127.0.0.1:8266/v1/chat/completions \
  -H "Authorization: Bearer <你的本地 API Key>" \
  -H "Content-Type: application/json" \
  -d '{"model":"glm-5.3-flash","messages":[{"role":"user","content":"hi"}],"stream":true}'
```

客户端侧配置示例：

- **API Base**: `http://127.0.0.1:8266/v1`
- **API Key**: 配置页中的 `sk-...`
- **Model**: `glm-5.3-flash` / `deepseek-v4-pro` / `qwen3.8-max` 等，或在配置页配置别名

## Linux CLI（cannbotcli）

在无图形界面的 Linux 服务器上，使用 `cannbotcli`（发布包内附 `cbc` 别名符号链接）完成配置并运行同一套核心。**CLI 与桌面版共用配置目录**（Linux: `~/.config/cn.hicann.cannbot-adapter`，含加密密钥 `.keyfile`，两边可互读配置）；注意两者不要同时运行（会抢占同一端口），CLI 修改配置后需重启 `serve` 才生效。CLI 不提供用量查询（用量仍会在后台记账，桌面端可查看同一份 `usage.db`）。

### 安装

从 [Releases](../../releases) 下载 `cannbotcli-linux-x86_64.tar.gz`（附 sha256 校验文件）：

```bash
tar xzf cannbotcli-linux-x86_64.tar.gz
sudo install -m 755 cannbotcli /usr/local/bin/   # 如需 cbc 别名: sudo install -m 755 cbc /usr/local/bin/
```

或从源码构建（仅需 Rust stable，无任何 GUI 依赖）：

```bash
cargo build --release -p cannbot-cli
# 产物: target/release/cannbotcli；可自行 ln -s 创建 cbc 别名
```

### 配置

```bash
cannbotcli config set vk <个人VK>        # 从公司/管理员处获取；改为 echo $VK | cannbotcli config set vk - 可避免进入 shell 历史
cannbotcli config set port 8266          # 可选，默认 8266
cannbotcli config set api-key <key>      # 可选；监听 0.0.0.0 时必须设置
cannbotcli config alias add gpt-4o glm-5.3-flash   # 可选：模型别名
cannbotcli config show                   # 查看当前配置（含敏感字段明文）
```

可配置项：`port`、`bind`（127.0.0.1 / 0.0.0.0）、`api-key`、`vk`、`vk-policy`（auto / builtin / personal / manual）、`manual-vk`（逗号分隔多个）。

### 运行

```bash
cannbotcli serve            # 前台运行本地代理，Ctrl-C / SIGTERM 优雅退出
cannbotcli status           # 检查 VK 认证与模型列表同步
cannbotcli models           # 列出可用模型与别名
```

systemd 服务示例（用户级，开机自启）：

```ini
# ~/.config/systemd/user/cannbot.service
[Unit]
Description=CANNBot adapter (OpenAI-compatible local proxy)
After=network-online.target

[Service]
ExecStart=/usr/local/bin/cannbotcli serve
Restart=on-failure
# 如需独立于桌面版的配置: Environment=... 并改用 ExecStart=/usr/local/bin/cannbotcli --config-dir /home/USER/.config/cannbot-cli serve

[Install]
WantedBy=default.target
```

```bash
systemctl --user daemon-reload && systemctl --user enable --now cannbot
journalctl --user -u cannbot -f          # 查看日志（日志同时输出到 stderr）
```

## 从源码构建

前置要求：Node.js ≥ 20、Rust stable、（Windows 构建 NSIS/MSI 需要 Windows 或直接用 CI）。

```bash
npm install
npm run tauri dev      # 开发调试
npm run tauri build    # 生产构建（产物在 src-tauri/target/release/bundle/）
```

> 注意：macOS 打 dmg 时不要设置 `http_proxy` 等代理环境变量，会干扰 `hdiutil`。

## 发布

推 tag 触发 GitHub Actions（`.github/workflows/release.yml`）自动构建并上传：

```bash
git tag v0.1.0 && git push origin v0.1.0
```

产物：macOS dmg（aarch64）、Windows msi + nsis、Linux `cannbotcli-linux-x86_64.tar.gz`。

## 架构

```
┌─ cannbot-core（UI 无关核心）────────────────────────────┐
│  ├─ axum HTTP server → /v1/chat/completions (SSE 透传)    │
│  │                       /v1/models / /health             │
│  ├─ VK 认证         → VK 换 JWT (100 年), 401 自动重认证   │
│  ├─ VK 策略/fallback → 内置优先, 配额类错误按策略切换       │
│  ├─ models sync     → 定时拉取网关模型列表与 VK 映射        │
│  └─ config/secret   → 配置持久化 + AES-256-GCM 加密        │
├──────────────────────────────────────────────────────────┤
│ cannbot-adapter (Tauri 2 桌面壳)   cannbotcli (CLI 壳)    │
│  托盘/配置页/用量图表/自启动          serve/config/status    │
└──────────────────────────────────────────────────────────┘
```

Cargo workspace：`core/`（`cannbot-core`，无 GUI 依赖，全部业务逻辑）为两个前端共享；`src-tauri/`（桌面 App：`tray.rs` 托盘、`logging.rs` 日志、`lib.rs` 组装 IPC 命令并转发核心事件）；`cli/`（`cannbotcli`：clap 命令行，前台运行代理 + 配置管理）。核心模块详见 `core/src/`：`config.rs`（配置与校验）、`secret.rs`（VK 加密）、`jwt.rs`（VK→JWT 认证）、`upstream.rs`（网关客户端/模型注册表）、`proxy.rs`（本地 OpenAI 端点与 fallback 主循环）、`usage.rs`（用量记账）、`state.rs`（共享状态/事件通知）。

前端（`src/`）：`App.svelte` 仅做编排（加载/页签/事件监听/保存）；`components/` 按卡片拆分（StatusCard / AccessInfoCard / ServerSettingsCard / ModelListCard / AliasCard / UsageCard / Toast）；`lib/` 为纯逻辑（`ipc.ts` IPC 封装、`config-form.svelte.ts` 配置表单状态与 VK 掩码哨兵、`usage.ts` 用量聚合、`toast.svelte.ts` 全局提示、`format.ts` / `clipboard.ts` 工具）；`styles/shared.css` 承载跨组件通用样式。

认证链路（用户唯一凭据是个人 VK）：

```
个人 VK ──authenticate(type=vk, mac=设备ID)──▶ JWT(≈100年)
   │                                            │
   │                                     GET /api/models/personal
   │                                            │ (条目内嵌 VK, 自动发现内置 VK)
   ▼                                            ▼
聊天 = Authorization: Bearer <JWT> + x-api-vkey: <VK>
```

## 已知限制

1. **VK fallback 判定规则待校准**：当前按「429 一律重试下一 VK；403/402 且错误体含 quota/配额 关键词」判定，配额耗尽的真实响应样本尚缺，日志中有完整记录，后续可按样本收紧。
2. **流式请求仅首包前可 fallback**：SSE 已开始输出后无法切换 VK 重试。
3. **Usage 记账依赖响应中的 usage 字段**：上游不带 usage 的响应无法记账；超过 256KB 的响应体跳过记账（防内存膨胀）；客户端在响应结束前主动断开时，本次请求不计入（上游已消耗，本地未记账）。
4. `mac`（设备标识）参数由适配器生成随机 MAC 格式 ID 并持久化（隐私考虑，不读真实硬件 MAC）。
5. Windows 下 `.keyfile`（VK 加密盐）仅依赖目录权限保护（0600 仅在 unix 生效），无额外 ACL 收紧。

## 配置文件

`macOS: ~/Library/Application Support/cn.hicann.cannbot-adapter/config.json`
`Linux: ~/.config/cn.hicann.cannbot-adapter/config.json`
`Windows: %APPDATA%\cn.hicann.cannbot-adapter\config.json`

VK 字段以 `enc:v1:` 前缀加密存储；解密密钥由同目录 `.keyfile`（0600）随机盐 + 安装路径派生，绑定本机。用量数据存于同目录 `usage.db`（SQLite）。桌面版与 CLI 共用此目录（CLI 可用 `--config-dir` 指定独立目录）。
