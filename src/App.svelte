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
    fetchModelsCmd,
    getUsage,
    clearUsage,
    type AppConfig,
    type ModelInfo,
    type StatusView,
    type UsageRow,
    type VkPolicy,
  } from "./lib/ipc";

  // ---- reactive state ----
  let loading = $state(true);
  let loaded = $state(false);
  let tab = $state<"general" | "usage">("general");
  let status = $state<StatusView | null>(null);
  let models = $state<ModelInfo[]>([]);
  let autostart = $state(false);

  let port = $state(8266);
  let bind = $state("127.0.0.1");
  let apiKey = $state("");
  let showKey = $state(false);
  let userVk = $state("");
  let showVk = $state(false);
  let vkEdited = $state(false);
  let vkPolicy = $state<VkPolicy>("auto");
  let manualVk = $state("");
  let manualVkEdited = $state(false);
  let aliasRows = $state<{ alias: string; model: string }[]>([]);

  // usage tab
  let usageRows = $state<UsageRow[]>([]);
  let usageDays = $state(30);
  let usageLoading = $state(false);
  let usageView = $state<"chart" | "byDate" | "byModel">("chart");

  let original = $state("");
  let saving = $state(false);
  let testing = $state(false);
  let toast = $state<{ kind: "ok" | "err"; text: string } | null>(null);

  // per-model liveness probe: model -> { running, ok, message }
  let probes = $state<Record<string, { running: boolean; ok: boolean; message: string }>>({});

  // histogram hover tooltip: model name + that day's token total
  let hoverTip = $state<{ model: string; tokens: string; left: number; top: number } | null>(null);

  function showSegTip(e: MouseEvent, model: string, tokens: number) {
    const seg = e.currentTarget as HTMLElement;
    const bars = seg.closest(".chart-bars") as HTMLElement | null;
    if (!bars) return;
    const segRect = seg.getBoundingClientRect();
    const barsRect = bars.getBoundingClientRect();
    // Coordinates in the chart-bars content space (compensate horizontal scroll).
    const centerX = segRect.left + segRect.width / 2 - barsRect.left + bars.scrollLeft;
    const centerY = segRect.top + segRect.height / 2 - barsRect.top;
    // Clamp so the tooltip never leaves the (clipping) scroll container.
    const left = Math.max(90, Math.min(centerX, barsRect.width + bars.scrollLeft - 90));
    const top = Math.max(26, Math.min(centerY, barsRect.height - 26));
    hoverTip = {
      model,
      tokens: `${fmtTokens(tokens)} tokens`,
      left,
      top,
    };
  }

  let toastTimer: ReturnType<typeof setTimeout> | undefined;

  const VK_MASK_SENTINEL = "**********";

  const dirty = $derived(loaded && serialize(buildConfig()) !== original);

  // ---- helpers ----
  function buildConfig(): AppConfig {
    const aliases: Record<string, string> = {};
    for (const row of aliasRows) {
      const a = row.alias.trim();
      const m = row.model.trim();
      if (a && m) aliases[a] = m;
    }
    // The backend never sends the plaintext VK; only forward a new value the
    // user actually typed. An untouched (sentinel) field means "keep stored".
    const userVkOut = vkEdited ? (userVk.trim() === "" ? null : userVk.trim()) : VK_MASK_SENTINEL;
    const manualVkOut = manualVkEdited
      ? manualVk.trim() === ""
        ? null
        : manualVk.trim()
      : manualVk
        ? VK_MASK_SENTINEL
        : null;
    return {
      port: Number(port) || 0,
      bind,
      api_key: apiKey.trim() === "" ? null : apiKey.trim(),
      user_vk: userVkOut,
      device_id: config_device_id,
      vk_policy: vkPolicy,
      manual_vk: manualVkOut,
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
    vkEdited = false;
    showVk = false;
    config_device_id = cfg.device_id ?? null;
    vkPolicy = cfg.vk_policy ?? "auto";
    manualVk = cfg.manual_vk ?? "";
    manualVkEdited = false;
    aliasRows = Object.entries(cfg.model_aliases).map(([alias, model]) => ({
      alias,
      model,
    }));
    // Reload the model list so per-row aliases reflect the config.
    void refreshModels();
    original = serialize(buildConfig());
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

  async function refreshModels() {
    models = await getModels().catch(() => []);
  }

  async function refreshUsage() {
    usageLoading = true;
    try {
      usageRows = await getUsage(usageDays);
    } catch (e) {
      showToast("err", String(e));
    } finally {
      usageLoading = false;
    }
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

  async function fetchModels() {
    testing = true;
    try {
      await fetchModelsCmd();
      await refreshStatus();
      showToast("ok", "模型列表已刷新");
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

  /// Set the alias for a real model row (fused alias editing in the model list).
  function setModelAlias(model: string, alias: string) {
    const trimmed = alias.trim();
    if (!trimmed) {
      // Clearing: drop the alias bound to this model.
      aliasRows = aliasRows.filter((r) => r.model !== model);
      return;
    }
    // Conflict: the alias already points at a different model.
    const conflict = aliasRows.find((r) => r.alias === trimmed && r.model !== model);
    if (conflict) {
      showToast("err", `别名「${trimmed}」已被模型 ${conflict.model} 占用，请先删除原有映射`);
      return;
    }
    // Drop any previous alias pointing at this model, then set the new one.
    aliasRows = aliasRows.filter((r) => r.model !== model);
    aliasRows.push({ alias: trimmed, model });
  }

  function modelAlias(model: string): string {
    return aliasRows.find((r) => r.model === model)?.alias ?? "";
  }

  /// Probe one model through the local adapter endpoint (minimal chat call).
  async function probeModel(model: string) {
    if (probes[model]?.running) return;
    probes[model] = { running: true, ok: false, message: "" };
    const base = status?.api_base ?? `http://127.0.0.1:${port}/v1`;
    const key = apiKey.trim();
    try {
      const resp = await fetch(`${base}/chat/completions`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          ...(key ? { Authorization: `Bearer ${key}` } : {}),
        },
        body: JSON.stringify({
          model,
          messages: [{ role: "user", content: "hi" }],
          max_tokens: 1,
        }),
        signal: AbortSignal.timeout(30_000),
      });
      if (resp.ok) {
        probes[model] = { running: false, ok: true, message: "正常" };
      } else {
        const text = await resp.text();
        let msg = `HTTP ${resp.status}`;
        try {
          const j = JSON.parse(text);
          msg = j?.error?.message ? `${msg}: ${j.error.message}` : msg;
        } catch {
          /* keep status-only message */
        }
        probes[model] = { running: false, ok: false, message: msg };
      }
    } catch (e) {
      const msg = e instanceof DOMException && e.name === "TimeoutError" ? "请求超时" : `连接失败: ${e}`;
      probes[model] = { running: false, ok: false, message: msg };
    }
  }

  async function onClearUsage() {
    try {
      await clearUsage();
      await refreshUsage();
      showToast("ok", "已清空用量记录");
    } catch (e) {
      showToast("err", String(e));
    }
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

  function fmtTokens(n: number): string {
    return n.toLocaleString();
  }

  async function copyAllModels() {
    await copyText(models.map((m) => m.model).join("\n"));
  }

  // usage aggregation for the two views
  const usageByDate = $derived(
    [...usageRows.reduce((map, r) => {
        const cur = map.get(r.date) ?? { date: r.date, requests: 0, input: 0, output: 0, models: 0 };
        cur.requests += r.requests;
        cur.input += r.input_tokens;
        cur.output += r.output_tokens;
        cur.models += 1;
        map.set(r.date, cur);
        return map;
      }, new Map<string, { date: string; requests: number; input: number; output: number; models: number }>()).values()].sort((a, b) => b.date.localeCompare(a.date)),
  );

  const usageByModel = $derived(
    [...usageRows.reduce((map, r) => {
        const cur = map.get(r.model) ?? { model: r.model, requests: 0, input: 0, output: 0, days: 0 };
        cur.requests += r.requests;
        cur.input += r.input_tokens;
        cur.output += r.output_tokens;
        cur.days += 1;
        map.set(r.model, cur);
        return map;
      }, new Map<string, { model: string; requests: number; input: number; output: number; days: number }>()).values()].sort((a, b) => b.output + b.input - (a.output + a.input)),
  );

  const usageTotal = $derived({
    requests: usageRows.reduce((s, r) => s + r.requests, 0),
    input: usageRows.reduce((s, r) => s + r.input_tokens, 0),
    output: usageRows.reduce((s, r) => s + r.output_tokens, 0),
  });

  /// Color palette for model segments in the histogram.
  const MODEL_COLORS = ["#559bf2", "#32bd76", "#f2a552", "#b06ce6", "#e66a8a", "#5bc8d9", "#a0a84a", "#8a8f9c"];

  /// Daily series for the histogram: one bar per day (zero-filled), each bar
  /// stacked by model with per-model token totals.
  const usageDaily = $derived.by(() => {
    const days: {
      date: string;
      label: string;
      segments: { model: string; tokens: number }[];
      total: number;
    }[] = [];
    const colorOf = new Map<string, string>();
    if (usageRows.length === 0) return { days, colorOf };

    // model -> color index (stable across days, ordered by total desc)
    const modelTotals = new Map<string, number>();
    for (const r of usageRows) {
      modelTotals.set(r.model, (modelTotals.get(r.model) ?? 0) + r.input_tokens + r.output_tokens);
    }
    const models = [...modelTotals.entries()].sort((a, b) => b[1] - a[1]).map(([m]) => m);
    for (let i = 0; i < models.length; i++) {
      colorOf.set(models[i], MODEL_COLORS[i % MODEL_COLORS.length]);
    }

    // date -> model -> tokens
    const byDate = new Map<string, Map<string, number>>();
    for (const r of usageRows) {
      let m = byDate.get(r.date);
      if (!m) {
        m = new Map();
        byDate.set(r.date, m);
      }
      m.set(r.model, (m.get(r.model) ?? 0) + r.input_tokens + r.output_tokens);
    }
    const dates = [...byDate.keys()].sort();
    const start = new Date(dates[0] + "T00:00:00");
    const end = new Date(dates[dates.length - 1] + "T00:00:00");
    const pad = (n: number) => String(n).padStart(2, "0");
    for (let d = new Date(start); d <= end; d.setDate(d.getDate() + 1)) {
      // Format in local time — toISOString() would shift back to UTC and
      // misattribute the day in non-UTC timezones.
      const iso = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
      const dayModels = byDate.get(iso);
      const segments = dayModels
        ? [...dayModels.entries()].map(([model, tokens]) => ({ model, tokens }))
        : [];
      days.push({
        date: iso,
        label: `${d.getMonth() + 1}/${d.getDate()}`,
        segments,
        total: segments.reduce((s, x) => s + x.tokens, 0),
      });
    }
    return { days, colorOf };
  });

  const usageMax = $derived(Math.max(1, ...usageDaily.days.map((d) => d.total)));

  /// Height % for a segment: proportional, but with a small floor so tiny
  /// models stay visible/hoverable when totals differ by orders of magnitude
  /// (e.g. 20M vs 100 tokens would otherwise render sub-pixel).
  const MIN_SEG_PCT = 3;
  function segHeightPct(tokens: number): number {
    const pct = (tokens / usageMax) * 100;
    return Math.max(pct, MIN_SEG_PCT);
  }

  function switchTab(t: "general" | "usage") {
    tab = t;
    if (t === "usage") {
      void refreshUsage();
    }
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
    <div class="header-actions">
      <button class="btn ghost" onclick={() => fetchModels()} disabled={testing}>
        {testing ? "获取中…" : "获取模型"}
      </button>
    </div>
  </header>

  {#if loading}
    <div class="card loading-card">加载中…</div>
  {:else}
    <nav class="tabs">
      <button class="tab" class:active={tab === "general"} onclick={() => switchTab("general")}>
        常规
      </button>
      <button class="tab" class:active={tab === "usage"} onclick={() => switchTab("usage")}>
        用量
      </button>
    </nav>

    {#if tab === "general"}
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
                内置 {status.vk.builtin ? "已获取" : "未获取"} · 个人 {status.vk.personal ? "已获取" : "未获取"}
              </span>
            {:else}
              <span class="muted">尚未获取</span>
            {/if}
          </div>
        </div>

        <div class="vk-block">
          <div class="subhead">个人 VK</div>
          <p class="hint">
            从公司/管理员处获取的个人虚拟密钥（vk- 开头），加密存储于本机；适配器用它完成网关认证并自动换取 JWT。内置 VK 会在认证后自动发现。
          </p>
          <div class="inline-group">
            <input
              class="fill mono-input"
              type={showVk ? "text" : "password"}
              placeholder="vk-..."
              spellcheck="false"
              autocomplete="off"
              value={userVk}
              oninput={(e) => {
                userVk = e.currentTarget.value;
                vkEdited = true;
              }}
            />
            {#if !vkEdited && userVk}
              <button class="btn ghost" onclick={() => { vkEdited = true; userVk = ""; }}>修改</button>
            {/if}
            <button class="btn ghost" onclick={() => (showVk = !showVk)}>
              {showVk ? "隐藏" : "显示"}
            </button>
            {#if vkEdited}
              <button
                class="btn ghost"
                onclick={() => {
                  userVk = VK_MASK_SENTINEL;
                  vkEdited = false;
                }}>撤销</button
              >
            {/if}
          </div>
        </div>
      </section>

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

        <div class="field-row">
          <span class="field-label">VK 策略</span>
          <div class="inline-group">
            <select bind:value={vkPolicy}>
              <option value="auto">自动 fallback（内置优先，配额不足切个人）</option>
              <option value="builtin_only">仅内置 VK（3000 万 token/日，仅 3 个 flash 模型）</option>
              <option value="personal_only">仅个人 VK（1 亿/日 + 10 亿/月，全量模型）</option>
              <option value="manual">手动指定</option>
            </select>
          </div>
        </div>
        {#if vkPolicy === "manual"}
          <div class="field-row">
            <span class="field-label">手动 VK</span>
            <div class="inline-group">
              <input
                class="fill mono-input"
                placeholder="vk-xxx（多个用英文逗号分隔，按序尝试）"
                spellcheck="false"
                autocomplete="off"
                value={manualVk}
                oninput={(e) => {
                  manualVk = e.currentTarget.value;
                  manualVkEdited = true;
                }}
              />
              {#if !manualVkEdited && manualVk}
                <button
                  class="btn ghost"
                  onclick={() => {
                    manualVk = "";
                    manualVkEdited = true;
                  }}>清空</button
                >
              {/if}
            </div>
          </div>
        {/if}
        {#if status?.vk.builtin || status?.vk.personal}
          <p class="muted small">
            自动获取到的 VK：内置 {status.vk.builtin ? "已获取" : "未获取"} · 个人 {status.vk.personal ? "已获取" : "未获取"}
          </p>
        {/if}
        <p class="hint">
          客户端以 OpenAI 格式访问：Authorization: ******;API Key&gt;。监听局域网（0.0.0.0）时必须设置
          API Key。
        </p>
      </section>

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
                    <span class="muted small nowrap">ctx {fmtK(m.context_length)}</span>
                  {/if}
                  {#if m.max_tokens}
                    <span class="muted small nowrap">max {fmtK(m.max_tokens)}</span>
                  {/if}
                {/if}
                {#if !m.alias_of}
                  <span class="alias-label">别名</span>
                  <input
                    class="alias-input"
                    placeholder="如 gpt-4o"
                    spellcheck="false"
                    value={modelAlias(m.model)}
                    onchange={(e) => setModelAlias(m.model, e.currentTarget.value)}
                  />
                {/if}
                <button
                  class="btn ghost probe-btn"
                  onclick={() => probeModel(m.model)}
                  disabled={probes[m.model]?.running}
                  title="通过本地接口发送一次最小请求，测试该模型是否可用"
                >
                  {probes[m.model]?.running ? "探测中…" : "探活"}
                </button>
                {#if probes[m.model] && !probes[m.model].running}
                  <span class="probe-result {probes[m.model].ok ? 'ok' : 'err'}">
                    {probes[m.model].ok ? "✓" : "✗"} {probes[m.model].message}
                  </span>
                {/if}
                <button class="btn ghost copy-btn" onclick={() => copyText(m.model)}>复制</button>
              </div>
            {/each}
          </div>
          <p class="hint">
            徽标表示该模型支持的 VK 通道：双通道模型任意策略可调；「仅内置」模型在「仅个人」策略下会收到网关 403。在行内「别名」框输入名称并保存后，客户端即可用别名调用该模型。
          </p>
        {/if}
      </section>

      {#if aliasRows.some((r) => !models.some((m) => m.model === r.model))}
        <section class="card">
          <h2>模型别名</h2>
          <p class="hint">
            以下别名未指向上方模型列表中的模型（列表未同步或目标已下线）。客户端请求 model =
            左侧别名时，自动替换为右侧真实模型名。
          </p>
          {#each aliasRows as row, i}
            {#if !models.some((m) => m.model === row.model)}
              <div class="alias-row">
                <input placeholder="别名，如 gpt-4o" spellcheck="false" bind:value={row.alias} />
                <span class="arrow">→</span>
                <input placeholder="真实模型，如 glm-5.3-flash" spellcheck="false" bind:value={row.model} />
                <button class="btn ghost danger" onclick={() => removeAlias(i)} aria-label="删除">✕</button>
              </div>
            {/if}
          {/each}
          <button class="btn ghost" onclick={addAlias}>+ 添加别名</button>
        </section>
      {/if}

      <footer class="actions">
        <span class="muted">{dirty ? "有未保存的修改" : ""}</span>
        <button class="btn primary" onclick={() => save()} disabled={!dirty || saving}>
          {saving ? "保存中…" : "保存"}
        </button>
      </footer>
    {:else if tab === "usage"}
      <section class="card">
        <div class="card-head">
          <h2>Token 用量</h2>
          <div class="usage-controls">
            <select bind:value={usageDays} onchange={() => refreshUsage()}>
              <option value={7}>近 7 天</option>
              <option value={30}>近 30 天</option>
              <option value={90}>近 90 天</option>
            </select>
            <button class="btn ghost" onclick={() => refreshUsage()}>刷新</button>
            <button class="btn ghost danger" onclick={() => onClearUsage()}>清空</button>
          </div>
        </div>

        <div class="usage-summary">
          <div class="usage-stat">
            <span class="label">请求数</span>
            <span class="mono">{fmtTokens(usageTotal.requests)}</span>
          </div>
          <div class="usage-stat">
            <span class="label">输入 Tokens</span>
            <span class="mono">{fmtTokens(usageTotal.input)}</span>
          </div>
          <div class="usage-stat">
            <span class="label">输出 Tokens</span>
            <span class="mono">{fmtTokens(usageTotal.output)}</span>
          </div>
        </div>

        <div class="usage-view-toggle">
          <button class="tab small" class:active={usageView === "chart"} onclick={() => (usageView = "chart")}>
            直方图
          </button>
          <button class="tab small" class:active={usageView === "byDate"} onclick={() => (usageView = "byDate")}>
            按日期
          </button>
          <button class="tab small" class:active={usageView === "byModel"} onclick={() => (usageView = "byModel")}>
            按模型
          </button>
        </div>

        {#if usageLoading}
          <p class="hint">加载中…</p>
        {:else if usageRows.length === 0}
          <p class="hint">暂无用量记录。统计数据来自本地记账（解析响应 usage 字段），从本版本启用后开始累计。</p>
        {:else if usageView === "chart"}
          <div class="usage-chart">
            <div class="chart-bars">
              {#each usageDaily.days as d (d.date)}
                <div class="chart-col">
                  <div class="chart-stack">
                    {#each d.segments as s (s.model)}
                      <!-- svelte-ignore a11y_no_static_element_interactions -->
                      <div
                        class="chart-seg"
                        role="img"
                        aria-label={`${s.model}: ${s.tokens} tokens`}
                        style={`height: ${segHeightPct(s.tokens)}%; background: ${usageDaily.colorOf.get(s.model)}`}
                        onmouseenter={(e) => showSegTip(e, s.model, s.tokens)}
                        onmouseleave={() => (hoverTip = null)}
                      ></div>
                    {/each}
                  </div>
                  <span class="chart-label">{d.label}</span>
                </div>
              {/each}
              {#if hoverTip}
                <div class="seg-tip" style={`left: ${hoverTip.left}px; top: ${hoverTip.top}px`}>
                  <span class="seg-tip-model">{hoverTip.model}</span>
                  <span class="seg-tip-tokens">{hoverTip.tokens}</span>
                </div>
              {/if}
            </div>
            <div class="chart-legend">
              {#each [...usageDaily.colorOf.entries()] as [model, color] (model)}
                <span class="legend-item"><span class="dot" style={`background: ${color}`}></span>{model}</span>
              {/each}
            </div>
          </div>
        {:else if usageView === "byDate"}
          <table class="usage-table">
            <thead>
              <tr>
                <th>日期</th>
                <th>请求数</th>
                <th>输入 Tokens</th>
                <th>输出 Tokens</th>
                <th>合计</th>
              </tr>
            </thead>
            <tbody>
              {#each usageByDate as r (r.date)}
                <tr>
                  <td class="mono">{r.date}</td>
                  <td>{fmtTokens(r.requests)}</td>
                  <td>{fmtTokens(r.input)}</td>
                  <td>{fmtTokens(r.output)}</td>
                  <td class="mono">{fmtTokens(r.input + r.output)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {:else}
          <table class="usage-table">
            <thead>
              <tr>
                <th>模型</th>
                <th>请求数</th>
                <th>输入 Tokens</th>
                <th>输出 Tokens</th>
                <th>合计</th>
              </tr>
            </thead>
            <tbody>
              {#each usageByModel as r (r.model)}
                <tr>
                  <td class="mono">{r.model}</td>
                  <td>{fmtTokens(r.requests)}</td>
                  <td>{fmtTokens(r.input)}</td>
                  <td>{fmtTokens(r.output)}</td>
                  <td class="mono">{fmtTokens(r.input + r.output)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
      </section>
    {/if}
  {/if}

  {#if toast}
    <div class="toast {toast.kind}">{toast.text}</div>
  {/if}
</main>

<style>
  main {
    max-width: 860px;
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
    gap: 10px;
    padding: 6px 0;
    border-bottom: 1px solid var(--border);
  }

  .model-row:last-child {
    border-bottom: none;
  }

  .model-name {
    white-space: nowrap;
  }

  .nowrap {
    white-space: nowrap;
  }

  .alias-label {
    color: var(--muted);
    font-size: 12px;
    white-space: nowrap;
  }

  .alias-input {
    width: 150px;
    flex: none;
    font-size: 13px;
    padding: 4px 8px;
  }

  .probe-btn {
    padding: 4px 10px;
    font-size: 12.5px;
  }

  .probe-result {
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 220px;
  }

  .probe-result.ok {
    color: var(--ok);
  }

  .probe-result.err {
    color: var(--danger);
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
  select {
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
  select:focus {
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

  .header-actions {
    display: flex;
    gap: 8px;
  }

  .tabs {
    display: flex;
    gap: 4px;
    border-bottom: 1px solid var(--border);
    padding-bottom: 0;
  }

  .tab {
    border: none;
    background: transparent;
    color: var(--muted);
    padding: 8px 16px;
    font-size: 14px;
    cursor: pointer;
    border-radius: 8px 8px 0 0;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
  }

  .tab:hover {
    color: var(--text);
  }

  .tab.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
    font-weight: 600;
  }

  .tab.small {
    padding: 5px 12px;
    font-size: 13px;
  }

  .vk-block {
    border-top: 1px dashed var(--border);
    padding-top: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .subhead {
    font-size: 13.5px;
    font-weight: 600;
  }

  .usage-controls {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .usage-summary {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
  }

  .usage-stat {
    flex: 1;
    min-width: 140px;
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 10px 14px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .usage-stat .label {
    color: var(--muted);
    font-size: 12.5px;
  }

  .usage-stat .mono {
    font-size: 18px;
  }

  .usage-view-toggle {
    display: flex;
    gap: 4px;
    border-bottom: 1px solid var(--border);
  }

  .usage-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13.5px;
  }

  .usage-table th {
    text-align: left;
    color: var(--muted);
    font-weight: 500;
    font-size: 12.5px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }

  .usage-table td {
    padding: 7px 8px;
    border-bottom: 1px solid var(--border);
  }

  .usage-table tr:last-child td {
    border-bottom: none;
  }

  .usage-table th:not(:first-child),
  .usage-table td:not(:first-child) {
    text-align: right;
  }

  .usage-chart {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .chart-bars {
    position: relative;
    display: flex;
    align-items: flex-end;
    gap: 3px;
    height: 180px;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
  }

  .chart-col {
    flex: 1;
    min-width: 18px;
    display: flex;
    flex-direction: column;
    align-items: center;
    height: 100%;
    gap: 4px;
    position: relative;
  }

  .chart-stack {
    flex: 1;
    width: 100%;
    max-width: 34px;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    border-radius: 4px 4px 0 0;
    overflow: hidden;
  }

  .chart-seg {
    width: 100%;
    transition: filter 0.15s;
  }

  .chart-seg:hover {
    filter: brightness(1.2);
  }

  /* Tooltip rendered by JS inside .chart-bars (absolutely positioned, two
     lines) — CSS ::after on the segment would be clipped by the stack's
     overflow:hidden and can't wrap lines. */
  .seg-tip {
    position: absolute;
    transform: translate(-50%, -50%);
    background: var(--card);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 6px 12px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    box-shadow: 0 4px 12px rgb(0 0 0 / 25%);
    z-index: 30;
    pointer-events: none;
    white-space: nowrap;
  }

  .seg-tip-model {
    font-size: 12.5px;
    font-weight: 600;
  }

  .seg-tip-tokens {
    font-size: 12px;
    color: var(--muted);
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  .chart-label {
    font-size: 10.5px;
    color: var(--muted);
    white-space: nowrap;
    max-width: 90px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .chart-legend {
    display: flex;
    gap: 14px;
    font-size: 12.5px;
    color: var(--muted);
  }

  .legend-item {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }

  .dot {
    width: 9px;
    height: 9px;
    border-radius: 3px;
    display: inline-block;
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

