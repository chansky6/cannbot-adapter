<script lang="ts">
  import { fmtTime } from "../lib/format";
  import { VK_MASK_SENTINEL, type ConfigForm } from "../lib/config-form.svelte";
  import type { StatusView } from "../lib/ipc";

  let { status, form }: { status: StatusView | null; form: ConfigForm } = $props();
</script>

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
        type={form.showVk ? "text" : "password"}
        placeholder="vk-..."
        spellcheck="false"
        autocomplete="off"
        value={form.userVk}
        oninput={(e) => {
          form.userVk = e.currentTarget.value;
          form.vkEdited = true;
        }}
      />
      {#if !form.vkEdited && form.userVk}
        <button class="btn ghost" onclick={() => { form.vkEdited = true; form.userVk = ""; }}>修改</button>
      {/if}
      <button class="btn ghost" onclick={() => (form.showVk = !form.showVk)}>
        {form.showVk ? "隐藏" : "显示"}
      </button>
      {#if form.vkEdited}
        <button
          class="btn ghost"
          onclick={() => {
            form.userVk = VK_MASK_SENTINEL;
            form.vkEdited = false;
          }}>撤销</button
        >
      {/if}
    </div>
  </div>
</section>

<style>
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

  .subhead {
    font-size: 13.5px;
    font-weight: 600;
  }

  .vk-block {
    border-top: 1px dashed var(--border);
    padding-top: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
</style>
