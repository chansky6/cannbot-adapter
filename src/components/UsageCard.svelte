<script lang="ts">
  import { onMount } from "svelte";
  import { clearUsage, getUsage } from "../lib/ipc";
  import type { UsageRow } from "../lib/ipc";
  import { fmtTokens } from "../lib/format";
  import { showToast } from "../lib/toast.svelte";
  import { usageByDate, usageByModel, usageTotal, usageDaily } from "../lib/usage";

  let usageRows = $state<UsageRow[]>([]);
  let usageDays = $state(30);
  let usageLoading = $state(false);
  let usageView = $state<"chart" | "byDate" | "byModel">("chart");

  // histogram hover tooltip: model name + that day's token total
  let hoverTip = $state<{ model: string; tokens: string; left: number; top: number } | null>(null);

  const byDate = $derived(usageByDate(usageRows));
  const byModel = $derived(usageByModel(usageRows));
  const total = $derived(usageTotal(usageRows));
  const daily = $derived(usageDaily(usageRows));
  const usageMax = $derived(Math.max(1, ...daily.days.map((d) => d.total)));

  /// Height % for a segment: proportional, but with a small floor so tiny
  /// models stay visible/hoverable when totals differ by orders of magnitude
  /// (e.g. 20M vs 100 tokens would otherwise render sub-pixel).
  const MIN_SEG_PCT = 3;
  function segHeightPct(tokens: number): number {
    const pct = (tokens / usageMax) * 100;
    return Math.max(pct, MIN_SEG_PCT);
  }

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

  async function onClearUsage() {
    try {
      await clearUsage();
      await refreshUsage();
      showToast("ok", "已清空用量记录");
    } catch (e) {
      showToast("err", String(e));
    }
  }

  // The component mounts when the usage tab is shown ({#if} in App.svelte),
  // which is when a reload was historically triggered.
  onMount(() => {
    void refreshUsage();
  });
</script>

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
      <span class="mono">{fmtTokens(total.requests)}</span>
    </div>
    <div class="usage-stat">
      <span class="label">输入 Tokens</span>
      <span class="mono">{fmtTokens(total.input)}</span>
    </div>
    <div class="usage-stat">
      <span class="label">输出 Tokens</span>
      <span class="mono">{fmtTokens(total.output)}</span>
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
        {#each daily.days as d (d.date)}
          <div class="chart-col">
            <div class="chart-stack">
              {#each d.segments as s (s.model)}
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <div
                  class="chart-seg"
                  role="img"
                  aria-label={`${s.model}: ${s.tokens} tokens`}
                  style={`height: ${segHeightPct(s.tokens)}%; background: ${daily.colorOf.get(s.model)}`}
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
        {#each [...daily.colorOf.entries()] as [model, color] (model)}
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
        {#each byDate as r (r.date)}
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
        {#each byModel as r (r.model)}
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

<style>
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
</style>
