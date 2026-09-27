<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import {
    getConfig,
    getModels,
    getStatus,
    getAutostart,
    setAutostart,
    saveConfig,
    regenerateApiKey,
    testConnection,
    type AppConfig,
    type ModelInfo,
    type StatusView,
    type TestResult,
    type VkPolicy,
  } from "./lib/ipc";

  // ---- reactive state ----
  let loading = $state(true);
  let loaded = $state(false);
  let status = $state<StatusView | null>(null);
  let models = $state<ModelInfo[]>([]);
  let autostart = $state(false);

  let port = $state(8266);
  let bind = $state("127.0.0.1");
  let apiKey = $state("");
  let showKey = $state(false);
  let userVk = $state("");
  let vkPolicy = $state<VkPolicy>("auto");
  let manualVk = $state("");
  let aliasRows = $state<{ alias: string; model: string }[]>([]);

  let original = $state("");
  let saving = $state(false);
  let testing = $state(false);
  let testResult = $state<TestResult | null>(null);
  let toast = $state<{ kind: "ok" | "err"; text: string } | null>(null);

  let toastTimer: ReturnType<typeof setTimeout> | undefined;

  const dirty = $derived(loaded && serialize(buildConfig()) !== original);

  // ---- helpers ----
  function buildConfig(): AppConfig {
    const aliases: Record<string, string> = {};
    for (const row of aliasRows) {
      const a = row.alias.trim();
      const m = row.model.trim();
      if (a && m) aliases[a] = m;
    }
    return {
      port: Number(port) || 0,
      bind,
      api_key: apiKey.trim() === "" ? null : apiKey.trim(),
      user_vk: userVk.trim() === "" ? null : userVk.trim(),
      device_id: config_device_id,
      vk_policy: vkPolicy,
      manual_vk: manualVk.trim() === "" ? null : manualVk.trim(),
      model_aliases: aliases,
    };
  }

  function serialize(c: AppConfig): string {
    const aliases = Object.keys(c.model_aliases)
      .sort()
      .map((k) => `${k}=${c.model_aliases[k]}`)
      .join(";");
    return JSON.stringify([
      c.port,
      c.bind,
      c.api_key,
      c.user_vk,
      c.vk_policy,
      c.manual_vk,
      aliases,
    ]);
  }

  // device_id is managed by the backend (stable per install); keep the
  // original value so saves don't clobber it.
  let config_device_id: string | null = null;

  function applyConfig(cfg: AppConfig) {
    port = cfg.port;
    bind = cfg.bind === "0.0.0.0" ? "0.0.0.0" : "127.0.0.1";
    apiKey = cfg.api_key ?? "";
    userVk = cfg.user_vk ?? "";
    config_device_id = cfg.device_id ?? null;
    vkPolicy = cfg.vk_policy ?? "auto";
    manualVk = cfg.manual_vk ?? "";
    aliasRows = Object.entries(cfg.model_aliases).map(([alias, model]) => ({
      alias,
      model,
    }));
    original = serialize(cfg);
    loaded = true;
  }

  async function reload() {
    const [cfg, st, au] = await Promise.all([
      getConfig(),
      getStatus(),
      getAutostart().catch(() => false),
    ]);
    applyConfig(cfg);
    status = st;
    autostart = au;
    loading = false;
  }

  async function refreshStatus() {
    status = await getStatus().catch(() => null);
    models = await getModels().catch(() => []);
  }

  function showToast(kind: "ok" | "err", text: string) {
    toast = { kind, text };
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), 3200);
  }

  async function save() {
    const cfg = buildConfig();
    if (!cfg.port || cfg.port < 1 || cfg.port > 65535) {
      showToast("err", "端口无效（1-65535）");
      return;
    }
    if (cfg.bind === "0.0.0.0" && !cfg.api_key) {
      showToast("err", "监听局域网（0.0.0.0）时必须设置本地 API Key");
      return;
    }
    if (cfg.vk_policy === "manual" && !cfg.manual_vk) {
      showToast("err", "VK 策略为「手动指定」时必须填写至少一个 VK");
      return;
    }
    saving = true;
    try {
      const r = await saveConfig(cfg);
      showToast("ok", r.restarted ? "已保存，服务已重启" : "已保存");
      await reload();
    } catch (e) {
      showToast("err", String(e));
    } finally {
      saving = false;
    }
  }

  async function regenKey() {
    try {
      await regenerateApiKey();
      showToast("ok", "已生成新 API Key 并保存");
      await reload();
    } catch (e) {
      showToast("err", String(e));
    }
  }

  async function onAutostartChange(e: Event) {
    const target = e.currentTarget as HTMLInputElement;
    const val = target.checked;
    try {
      await setAutostart(val);
      autostart = val;
      showToast("ok", val ? "已开启开机自启动" : "已关闭开机自启动");
    } catch (err) {
      autostart = !val;
      showToast("err", String(err));
    }
  }

  async function runTest() {
    testing = true;
    testResult = null;
    try {
      testResult = await testConnection();
    } catch (e) {
      showToast("err", String(e));
    } finally {
      testing = false;
    }
  }

  async function copyText(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      showToast("ok", "已复制到剪贴板");
    } catch {
      showToast("err", "复制失败");
    }
  }

  function addAlias() {
    aliasRows.push({ alias: "", model: "" });
  }

  function removeAlias(i: number) {
    aliasRows.splice(i, 1);
  }

  function fmtTime(unix: number | null): string {
    if (!unix) return "-";
    return new Date(unix * 1000).toLocaleTimeString();
  }

  function fmtK(n: number): string {
    if (n >= 1_000_000) {
      return `${(n / 1_000_000).toFixed(n % 1_000_000 === 0 ? 0 : 1)}M`;
    }
    if (n >= 1_000) {
      return `${Math.round(n / 1_000)}K`;
    }
    return String(n);
  }

  async function copyAllModels() {
    await copyText(models.map((m) => m.model).join("\n"));
  }

  // ---- lifecycle ----
  $effect(() => {
    const cleanups: Array<() => void> = [];
    void (async () => {
      await reload().catch(() => {
        loading = false;
      });
      for (const name of ["server-status", "jwt-status", "models-synced"] as const) {
        const un = await listen(name, () => refreshStatus());
        cleanups.push(un);
      }
    })();
    return () => cleanups.forEach((u) => u());
  });
</script>

<main>
  <header>
    <div class="title">
      <h1>CANNBot Adapter</h1>
      <p>将 CANNBot 网关适配为本地 OpenAI 兼容 API</p>
    </div>
    <button class="btn ghost" onclick={() => runTest()} disabled={testing}>
      {testing ? "测试中…" : "测试连接"}
    </button>
  </header>

  {#if loading}
    <div class="card loading-card">加载中…</div>
  {:else}
    <!-- 状态 -->
    <section class="card">
      <h2>运行状态</h2>
      <div class="status-grid">
        <div class="status-item">
          <span class="label">本地服务</span>
          {#if status?.server_running}
            <span class="badge ok">运行中</span>
            <span class="mono">{status.server_addr}</span>
          {:else}
            <span class="badge err">未运行</span>
          {/if}
          {#if status?.server_error}
            <span class="error-text">{status.server_error}</span>
          {/if}
        </div>
        <div class="status-item">
          <span class="label">VK 认证</span>
          {#if status?.jwt.ok}
            <span class="badge ok">已认证</span>
          {:else}
            <span class="badge err">未认证</span>
          {/if}
          <span class="muted">{status?.jwt.message}</span>
        </div>
        <div class="status-item">
          <span class="label">上游模型</span>
          {#if status?.models_count}
            <span class="mono">{status.models_count} 个</span>
            <span class="muted">同步于 {fmtTime(status.models_synced_at)}</span>
          {:else}
            <span class="muted">尚未同步</span>
          {/if}
        </div>
        <div class="status-item">
          <span class="label">虚拟密钥 (VK)</span>
          {#if status?.vk.builtin || status?.vk.personal}
            <span class="mono">
              内置 {status.vk.builtin ?? "未获取"} · 个人 {status.vk.personal ?? "未获取"}
            </span>
          {:else}
            <span class="muted">尚未同步</span>
          {/if}
        </div>
      </div>
      {#if testResult}
        <div class="test-result">
          <div>JWT：{testResult.jwt_ok ? "✓ 有效" : "✗ 无效"}</div>
          <div>模型列表：{testResult.models_count} 个</div>
          <div>
            试聊（{testResult.chat_model}，max_tokens=5）：
            {testResult.chat_ok ? "✓ 成功" : `✗ ${testResult.chat_message}`}
          </div>
        </div>
      {/if}
    </section>

    <!-- 接入信息 -->
    <section class="card">
      <h2>接入信息</h2>
      <div class="field-row">
        <span class="field-label">API Base</span>
        <div class="inline-group">
          <code class="mono fill">{status?.api_base ?? ""}</code>
          <button class="btn ghost" onclick={() => copyText(status?.api_base ?? "")}>
            复制
          </button>
        </div>
      </div>
      <div class="field-row">
        <span class="field-label">API Key</span>
        <div class="inline-group">
          <input
            class="fill"
            type={showKey ? "text" : "password"}
            placeholder="留空表示关闭本地鉴权"
            bind:value={apiKey}
          />
          <button class="btn ghost" onclick={() => (showKey = !showKey)}>
            {showKey ? "隐藏" : "显示"}
          </button>
          <button class="btn ghost" onclick={() => copyText(apiKey)}>复制</button>
          <button class="btn ghost" onclick={() => regenKey()}>重新生成</button>
        </div>
      </div>
      <p class="hint">
        客户端以 OpenAI 格式访问：Authorization: Bearer &lt;API Key&gt;。监听局域网（0.0.0.0）时必须设置
        API Key。
      </p>
    </section>

    <!-- 服务设置 -->
    <section class="card">
      <h2>服务设置</h2>
      <div class="field-row">
        <label for="port">端口</label>
        <input id="port" type="number" min="1" max="65535" bind:value={port} />
      </div>
      <div class="field-row">
        <label for="bind">监听地址</label>
        <select id="bind" bind:value={bind}>
          <option value="127.0.0.1">仅本机（127.0.0.1）</option>
          <option value="0.0.0.0">局域网（0.0.0.0，需 API Key）</option>
        </select>
      </div>
      <div class="field-row">
        <span class="field-label">开机自启动</span>
        <label class="toggle">
          <input type="checkbox" checked={autostart} onchange={onAutostartChange} />
          <span class="slider"></span>
        </label>
      </div>
    </section>

    <!-- 上游凭据 -->
    <section class="card">
      <h2>上游凭据</h2>

      <div class="subhead">个人 VK（必填）</div>
      <p class="hint">
        从公司/管理员处获取的个人虚拟密钥（vk-
        开头）。适配器用它完成网关认证并自动换取 JWT，无需安装 opencode；内置 VK
        会在认证后自动发现。
      </p>
      <div class="inline-group">
        <input
          class="fill mono-input"
          type="text"
          placeholder="vk-..."
          spellcheck="false"
          bind:value={userVk}
        />
        <button class="btn ghost" onclick={() => copyText(userVk)}>复制</button>
      </div>

      <div class="subhead">VK 策略（计费通道）</div>
      <p class="hint">
        认证后适配器自动发现账号下的内置 VK；此处选择请求消耗哪个配额池。
      </p>
      <div class="field-row">
        <label for="vkPolicy">策略</label>
        <select id="vkPolicy" bind:value={vkPolicy}>
          <option value="auto">自动 fallback（内置优先，配额不足切个人）</option>
          <option value="builtin_only">仅内置 VK（3000 万 token/日，仅 3 个 flash 模型）</option>
          <option value="personal_only">仅个人 VK（1 亿/日 + 10 亿/月，全量模型）</option>
          <option value="manual">手动指定</option>
        </select>
      </div>
      {#if vkPolicy === "manual"}
        <textarea
          rows="2"
          placeholder="vk-xxx（多个用英文逗号分隔，按序尝试）"
          spellcheck="false"
          bind:value={manualVk}
        ></textarea>
      {/if}
      {#if status?.vk.builtin || status?.vk.personal}
        <p class="muted small">
          自动获取到的 VK：内置 {status.vk.builtin ?? "未获取"} · 个人 {status.vk.personal ?? "未获取"}
        </p>
      {/if}
    </section>

    <!-- 可用模型 -->
    <section class="card">
      <div class="card-head">
        <h2>可用模型</h2>
        {#if models.length > 0}
          <button class="btn ghost" onclick={() => copyAllModels()}>复制全部</button>
        {/if}
      </div>
      {#if models.length === 0}
        <p class="hint">尚未从网关同步（启动后自动拉取，也可点击顶部「测试连接」触发）。</p>
      {:else}
        <div class="model-list">
          {#each models as m (m.model)}
            <div class="model-row">
              <span class="mono model-name">{m.model}</span>
              {#if m.alias_of}
                <span class="badge neutral">别名 → {m.alias_of}</span>
              {:else}
                {#if m.builtin && m.personal}
                  <span class="badge ok">双通道</span>
                {:else if m.builtin}
                  <span class="badge neutral">仅内置</span>
                {:else}
                  <span class="badge accent">仅个人</span>
                {/if}
                {#if m.context_length}
                  <span class="muted small">ctx {fmtK(m.context_length)}</span>
                {/if}
                {#if m.max_tokens}
                  <span class="muted small">max {fmtK(m.max_tokens)}</span>
                {/if}
              {/if}
              <button class="btn ghost copy-btn" onclick={() => copyText(m.model)}>复制</button>
            </div>
          {/each}
        </div>
        <p class="hint">徽标表示该模型支持的 VK 通道：双通道模型任意策略可调；「仅内置」模型在「仅个人」策略下会收到网关 403。</p>
      {/if}
    </section>

    <!-- 模型别名 -->
    <section class="card">
      <h2>模型别名</h2>
      <p class="hint">
        客户端请求 model = 左侧别名时，自动替换为右侧真实模型名（如 gpt-4o → glm-5.3-flash）；未命中的模型名原样透传。
      </p>
      {#each aliasRows as row, i}
        <div class="alias-row">
          <input placeholder="别名，如 gpt-4o" spellcheck="false" bind:value={row.alias} />
          <span class="arrow">→</span>
          <input placeholder="真实模型，如 glm-5.3-flash" spellcheck="false" bind:value={row.model} />
          <button class="btn ghost danger" onclick={() => removeAlias(i)} aria-label="删除">✕</button>
        </div>
      {/each}
      <button class="btn ghost" onclick={addAlias}>+ 添加别名</button>
    </section>

    <footer class="actions">
      <span class="muted">{dirty ? "有未保存的修改" : ""}</span>
      <button class="btn primary" onclick={() => save()} disabled={!dirty || saving}>
        {saving ? "保存中…" : "保存"}
      </button>
    </footer>
  {/if}

  {#if toast}
    <div class="toast {toast.kind}">{toast.text}</div>
  {/if}
</main>

<style>
  main {
    max-width: 720px;
    margin: 0 auto;
    padding: 20px 16px 96px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  header h1 {
    margin: 0;
    font-size: 20px;
  }

  header p {
    margin: 2px 0 0;
    color: var(--muted);
    font-size: 13px;
  }

  .card {
    background: var(--card);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .card h2 {
    margin: 0;
    font-size: 15px;
  }

  .card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .model-list {
    display: flex;
    flex-direction: column;
  }

  .model-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-radius: 8px;
  }

  .model-row:hover {
    background: var(--bg);
  }

  .model-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .badge.neutral {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .badge.accent {
    background: var(--warn-soft);
    color: var(--warn);
  }

  .copy-btn {
    padding: 3px 10px;
    font-size: 12.5px;
  }

  .subhead {
    font-size: 13px;
    font-weight: 600;
    color: var(--muted);
    margin-top: 2px;
  }

  .loading-card {
    color: var(--muted);
    text-align: center;
    padding: 40px;
  }

  .status-grid {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .status-item {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .label {
    min-width: 110px;
    color: var(--muted);
  }

  .badge {
    padding: 1px 10px;
    border-radius: 999px;
    font-size: 12px;
  }

  .badge.ok {
    background: var(--ok-soft);
    color: var(--ok);
  }

  .badge.err {
    background: var(--danger-soft);
    color: var(--danger);
  }

  .error-text {
    color: var(--danger);
    font-size: 13px;
    width: 100%;
  }

  .test-result {
    border-top: 1px dashed var(--border);
    padding-top: 8px;
    font-size: 13px;
    color: var(--muted);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .field-row {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }

  .field-row > .field-label:first-child {
    min-width: 76px;
    color: var(--muted);
  }

  .inline-group {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
  }

  input,
  select,
  textarea {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    color: var(--text);
    padding: 7px 10px;
    font-size: 14px;
    font-family: inherit;
    outline: none;
    min-width: 0;
  }

  input:focus,
  select:focus,
  textarea:focus {
    border-color: var(--accent);
  }

  input[type="number"] {
    width: 120px;
  }

  .fill {
    flex: 1;
    width: 100%;
  }

  code.mono {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 7px 10px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 13px;
  }

  .mono-input {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12.5px;
  }

  .muted {
    color: var(--muted);
  }

  .small {
    font-size: 12px;
    margin: 0;
  }

  .hint {
    color: var(--muted);
    font-size: 12.5px;
    margin: 0;
  }

  textarea {
    resize: vertical;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 12.5px;
  }

  .alias-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .alias-row input:first-child {
    flex: 1;
  }

  .alias-row input:last-of-type {
    flex: 1.6;
  }

  .arrow {
    color: var(--muted);
  }

  .btn {
    border: 1px solid var(--border);
    background: var(--card);
    color: var(--text);
    border-radius: 8px;
    padding: 7px 14px;
    font-size: 13.5px;
    cursor: pointer;
    white-space: nowrap;
  }

  .btn:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn.ghost {
    background: transparent;
  }

  .btn.danger:hover:not(:disabled) {
    border-color: var(--danger);
    color: var(--danger);
  }

  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
    padding: 8px 26px;
  }

  .btn.primary:hover:not(:disabled) {
    filter: brightness(1.08);
    color: #fff;
  }

  .actions {
    position: fixed;
    bottom: 0;
    left: 0;
    right: 0;
    display: flex;
    align-items: center;
    justify-content:
      flex-end;
    gap: 12px;
    padding: 12px 24px;
    background: color-mix(in srgb, var(--bg) 88%, transparent);
    backdrop-filter: blur(8px);
    border-top: 1px solid var(--border);
  }

  /* toggle switch */
  .toggle {
    position: relative;
    display: inline-block;
    width: 42px;
    height: 24px;
    cursor: pointer;
  }

  .toggle input {
    opacity: 0;
    width: 0;
    height: 0;
    position: absolute;
  }

  .toggle .slider {
    position: absolute;
    inset: 0;
    background: var(--border);
    border-radius: 999px;
    transition: 0.2s;
  }

  .toggle .slider::before {
    content: "";
    position: absolute;
    width: 18px;
    height: 18px;
    left: 3px;
    top: 3px;
    background: var(--card);
    border-radius: 50%;
    transition: 0.2s;
  }

  .toggle input:checked + .slider {
    background: var(--accent);
  }

  .toggle input:checked + .slider::before {
    transform: translateX(18px);
  }

  .toast {
    position: fixed;
    bottom: 64px;
    left: 50%;
    transform: translateX(-50%);
    padding: 9px 18px;
    border-radius: 10px;
    font-size: 13.5px;
    box-shadow: 0 4px 16px rgb(0 0 0 / 18%);
    z-index: 10;
    max-width: 80vw;
  }

  .toast.ok {
    background: var(--ok-soft);
    color: var(--ok);
    border: 1px solid var(--ok);
  }

  .toast.err {
    background: var(--danger-soft);
    color: var(--danger);
    border: 1px solid var(--danger);
  }
</style>
