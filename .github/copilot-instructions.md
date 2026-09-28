# cannbot-adapter — Copilot 工作指南

Tauri 2 桌面应用（Rust 核心 + Svelte 5 WebView 配置页），把 CANNBot 网关适配为本地 OpenAI 兼容 API（默认 `http://127.0.0.1:8266/v1`），系统托盘常驻，支持 macOS / Windows。UI 文案与用户可见错误消息均为中文。

## 常用命令

```bash
npm install                  # 前端依赖
npm run tauri dev            # 开发调试（Rust + 前端 HMR；需要 Node ≥ 20 + Rust stable）
npm run check                # svelte-check 前端类型检查（改动前端后必须通过）
npm run build                # 仅构建前端（vite）
npm run tauri build          # 生产构建安装包（产物在 src-tauri/target/release/bundle/）

cd src-tauri && cargo test   # Rust 测试（目前集中在 usage.rs / secret.rs）
cd src-tauri && cargo test extracts_json_usage   # 运行单个测试
cd src-tauri && cargo check  # Rust 快速校验
```

- macOS 打 dmg 时不要设置 `http_proxy` 等代理环境变量（会干扰 `hdiutil`）。
- 发布：推 tag `v*` 触发 GitHub Actions（macOS aarch64 dmg + Windows msi/nsis）。应用未签名公证，macOS 用户需 `xattr -cr /Applications/cannbot-adapter.app`。

## 架构

前后端两层进程，**前端只是配置 UI，所有业务逻辑在 Rust**。

### Rust 核心（`src-tauri/src/`）

- `proxy.rs` — axum 本地 HTTP 服务（`/v1/chat/completions` SSE 透传、`/v1/models`、`/health`）+ **VK fallback 主循环**：按策略取候选 VK 依次尝试；401 重新认证 JWT 后原 VK 重试一次；配额类错误切下一 VK。注意 SSE 已开始输出后无法 fallback。配置保存会重启此服务（graceful shutdown 200ms 后 abort，防 SSE 长连接占用端口）。
- `jwt.rs` — 个人 VK 换长效 JWT（约 100 年），`auth_lock` 防并发重复认证，401 自动重认证。
- `upstream.rs` — 网关客户端与模型注册表；模型列表每 10 分钟同步（首成功前每 30s 重试）；内置 VK 从模型条目自动发现（`vks_for` 返回 builtin 优先的候选序）。
- `secret.rs` — VK 落盘加密：AES-256-GCM、`enc:v1:` 前缀，密钥由同目录 `.keyfile` 随机盐 + 安装路径派生（绑定本机）。
- `usage.rs` — SQLite 用量记账（`usage.db`，一行一个 (day, model)），从响应 `usage` 字段提取；超过 256KB 的响应体跳过记账。
- `config.rs` — 配置持久化（`VkPolicy` 枚举：auto / builtin_only / personal_only / manual）；`state.rs` — `SharedState`（一组 RwLock，经 `tauri::State<Shared>` 注入）。
- `lib.rs` — 组装与 IPC 命令注册（`generate_handler!`），以及各 `*View` 视图结构体。

### 前端（`src/`）

- `App.svelte` 只做编排（加载 / 页签 / 事件监听 / 保存校验）；`components/` 按卡片拆分组件；`lib/` 纯逻辑模块；`styles/shared.css` 跨组件通用样式（组件私有样式留在各自 `<style>`）。
- 前后端通信：Tauri IPC（全部封装在 `lib/ipc.ts`，命令名与 `lib.rs` 的 `generate_handler!` 一一对应）+ 后端事件 `server-status` / `jwt-status` / `models-synced`（App.svelte 监听后刷新状态与模型列表）。

### 认证链路

个人 VK（用户唯一上游凭据）→ `/api/auth/authenticate` 换 JWT；网关请求带 JWT Bearer + `x-api-vkey: <VK>`。

## 关键约定

- **VK 明文永不出后端**：IPC 返回 `MASK_SENTINEL`（`"**********"`）。前端 `ConfigForm`（`lib/config-form.svelte.ts`）的哨兵往返规则必须保持：字段未编辑 → 回传哨兵（保留存储值）；编辑后清空 → 回传 null（删除）。修改配置表单时勿破坏此语义。
- **视图结构体脱敏**：发给前端的 `*View`（StatusView / JwtStatusView / VkStatusView）绝不包含 JWT 或 VK 明文。
- **Svelte 5 runes**：统一用 `$state` / `$derived` / `$props` / `$effect`，不用 legacy 语法；组件外需要响应式逻辑时放 `.svelte.ts` 文件。配置表单状态集中在 `ConfigForm` 类，通过 props 下发、子组件直接绑定其字段。
- `device_id`（设备标识，MAC 格式）由后端生成并持久化，前端保存时原样带回，不要在前端生成或修改。
- commit 风格：简短前缀 + 中文/英文描述（如 `feat: …`、`ci: …`、`chore: …`）。
