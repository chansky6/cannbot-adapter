<script lang="ts">
  import { copyText } from "../lib/clipboard";
  import { showToast } from "../lib/toast.svelte";
  import { regenerateApiKey } from "../lib/ipc";
  import type { ConfigForm } from "../lib/config-form.svelte";
  import type { StatusView } from "../lib/ipc";

  let { status, form, onSaved }: { status: StatusView | null; form: ConfigForm; onSaved: () => void } = $props();

  async function regenKey() {
    try {
      await regenerateApiKey();
      showToast("ok", "已生成新 API Key 并保存");
      onSaved();
    } catch (e) {
      showToast("err", String(e));
    }
  }
</script>

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
        type={form.showKey ? "text" : "password"}
        placeholder="留空表示关闭本地鉴权"
        bind:value={form.apiKey}
      />
      <button class="btn ghost" onclick={() => (form.showKey = !form.showKey)}>
        {form.showKey ? "隐藏" : "显示"}
      </button>
      <button class="btn ghost" onclick={() => copyText(form.apiKey)}>复制</button>
      <button class="btn ghost" onclick={() => regenKey()}>重新生成</button>
    </div>
  </div>

  <div class="field-row">
    <span class="field-label">VK 策略</span>
    <div class="inline-group">
      <select bind:value={form.vkPolicy}>
        <option value="auto">自动 fallback（内置优先，配额不足切个人）</option>
        <option value="builtin_only">仅内置 VK（3000 万 token/日，仅 3 个 flash 模型）</option>
        <option value="personal_only">仅个人 VK（1 亿/日 + 10 亿/月，全量模型）</option>
        <option value="manual">手动指定</option>
      </select>
    </div>
  </div>
  {#if form.vkPolicy === "manual"}
    <div class="field-row">
      <span class="field-label">手动 VK</span>
      <div class="inline-group">
        <input
          class="fill mono-input"
          placeholder="vk-xxx（多个用英文逗号分隔，按序尝试）"
          spellcheck="false"
          autocomplete="off"
          value={form.manualVk}
          oninput={(e) => {
            form.manualVk = e.currentTarget.value;
            form.manualVkEdited = true;
          }}
        />
        {#if !form.manualVkEdited && form.manualVk}
          <button
            class="btn ghost"
            onclick={() => {
              form.manualVk = "";
              form.manualVkEdited = true;
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
    客户端以 OpenAI 格式访问：Authorization: Bearer &lt;API Key&gt;。监听局域网（0.0.0.0）时必须设置
    API Key。
  </p>
</section>
