<script lang="ts">
  import { fmtK } from "../lib/format";
  import { copyText } from "../lib/clipboard";
  import type { ConfigForm } from "../lib/config-form.svelte";
  import type { ModelInfo, StatusView } from "../lib/ipc";

  let {
    models,
    status,
    form,
  }: { models: ModelInfo[]; status: StatusView | null; form: ConfigForm } = $props();

  // per-model liveness probe: model -> { running, ok, message }
  let probes = $state<Record<string, { running: boolean; ok: boolean; message: string }>>({});

  /// Probe one model through the local adapter endpoint (minimal chat call).
  async function probeModel(model: string) {
    if (probes[model]?.running) return;
    probes[model] = { running: true, ok: false, message: "" };
    const base = status?.api_base ?? `http://127.0.0.1:${form.port}/v1`;
    const key = form.apiKey.trim();
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

  async function copyAllModels() {
    await copyText(models.map((m) => m.model).join("\n"));
  }
</script>

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
              value={form.modelAlias(m.model)}
              onchange={(e) => form.setModelAlias(m.model, e.currentTarget.value)}
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

<style>
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

  .copy-btn {
    padding: 3px 10px;
    font-size: 12.5px;
  }
</style>
