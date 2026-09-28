<script lang="ts">
  import { onMount } from "svelte";
  import { getAutostart, setAutostart } from "../lib/ipc";
  import { showToast } from "../lib/toast.svelte";
  import type { ConfigForm } from "../lib/config-form.svelte";

  let { form }: { form: ConfigForm } = $props();

  let autostart = $state(false);

  onMount(async () => {
    autostart = await getAutostart().catch(() => false);
  });

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
</script>

<section class="card">
  <h2>服务设置</h2>
  <div class="field-row">
    <label for="port">端口</label>
    <input id="port" type="number" min="1" max="65535" bind:value={form.port} />
  </div>
  <div class="field-row">
    <label for="bind">监听地址</label>
    <select id="bind" bind:value={form.bind}>
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
