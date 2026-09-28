<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import {
    getConfig,
    getModels,
    getStatus,
    saveConfig,
    fetchModelsCmd,
    type ModelInfo,
    type StatusView,
  } from "./lib/ipc";
  import { ConfigForm } from "./lib/config-form.svelte";
  import { showToast } from "./lib/toast.svelte";
  import StatusCard from "./components/StatusCard.svelte";
  import AccessInfoCard from "./components/AccessInfoCard.svelte";
  import ServerSettingsCard from "./components/ServerSettingsCard.svelte";
  import ModelListCard from "./components/ModelListCard.svelte";
  import AliasCard from "./components/AliasCard.svelte";
  import UsageCard from "./components/UsageCard.svelte";
  import Toast from "./components/Toast.svelte";

  // ---- reactive state ----
  let loading = $state(true);
  let loaded = $state(false);
  let tab = $state<"general" | "usage">("general");
  let status = $state<StatusView | null>(null);
  let models = $state<ModelInfo[]>([]);

  const form = new ConfigForm();

  let original = $state("");
  let saving = $state(false);
  let testing = $state(false);

  const dirty = $derived(loaded && form.serialize() !== original);

  // ---- data loading ----
  async function reload() {
    const [cfg, st] = await Promise.all([getConfig(), getStatus()]);
    form.apply(cfg);
    status = st;
    original = form.serialize();
    loaded = true;
  }

  async function refreshStatus() {
    status = await getStatus().catch(() => null);
    models = await getModels().catch(() => []);
  }

  async function refreshModels() {
    models = await getModels().catch(() => []);
  }

  async function reloadAndRefresh() {
    await reload();
    await refreshModels();
  }

  async function save() {
    const cfg = form.build();
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
    const realModels = new Set(models.map((m) => m.model).filter((m) => !models.find((x) => x.model === m && x.alias_of === m)));
    for (const [alias, target] of Object.entries(cfg.model_aliases)) {
      if (alias === target) {
        showToast("err", `别名「${alias}」不能指向自身`);
        return;
      }
      // A row whose model is itself an alias_of means alias-of-alias; real
      // gateway models are the ones listed without alias_of.
      if (realModels.has(alias)) {
        showToast("err", `别名「${alias}」与真实模型名冲突，会导致该模型无法直接调用，请换一个别名`);
        return;
      }
    }
    saving = true;
    try {
      const r = await saveConfig(cfg);
      showToast("ok", r.restarted ? "已保存，服务已重启" : "已保存");
      await reloadAndRefresh();
    } catch (e) {
      showToast("err", String(e));
    } finally {
      saving = false;
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

  // ---- lifecycle ----
  $effect(() => {
    const cleanups: Array<() => void> = [];
    void (async () => {
      try {
        await reload();
        await refreshModels();
      } catch {
        loading = false;
      } finally {
        loading = false;
      }
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
      <button class="tab" class:active={tab === "general"} onclick={() => (tab = "general")}>
        常规
      </button>
      <button class="tab" class:active={tab === "usage"} onclick={() => (tab = "usage")}>
        用量
      </button>
    </nav>

    {#if tab === "general"}
      <StatusCard {status} {form} />
      <AccessInfoCard {status} {form} onSaved={reloadAndRefresh} />
      <ServerSettingsCard {form} />
      <ModelListCard {models} {status} {form} />
      <AliasCard {form} {models} />

      <footer class="actions">
        <span class="muted">{dirty ? "有未保存的修改" : ""}</span>
        <button class="btn primary" onclick={() => save()} disabled={!dirty || saving}>
          {saving ? "保存中…" : "保存"}
        </button>
      </footer>
    {:else}
      <UsageCard />
    {/if}
  {/if}

  <Toast />
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

  .header-actions {
    display: flex;
    gap: 8px;
  }

  .loading-card {
    color: var(--muted);
    text-align: center;
    padding: 40px;
  }

  .actions {
    position: fixed;
    bottom: 0;
    left: 0;
    right: 0;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 12px;
    padding: 12px 24px;
    background: color-mix(in srgb, var(--bg) 88%, transparent);
    backdrop-filter: blur(8px);
    border-top: 1px solid var(--border);
  }
</style>
