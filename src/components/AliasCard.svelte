<script lang="ts">
  import type { ConfigForm } from "../lib/config-form.svelte";
  import type { ModelInfo } from "../lib/ipc";

  let { form, models }: { form: ConfigForm; models: ModelInfo[] } = $props();

  // Only show alias rows whose target model is not in the synced model list.
  const orphans = $derived(form.orphanRows(models.map((m) => m.model)));
</script>

{#if orphans.length > 0}
  <section class="card">
    <h2>模型别名</h2>
    <p class="hint">
      以下别名未指向上方模型列表中的模型（列表未同步或目标已下线）。客户端请求 model =
      左侧别名时，自动替换为右侧真实模型名。
    </p>
    {#each orphans as row (row)}
      <div class="alias-row">
        <input placeholder="别名，如 gpt-4o" spellcheck="false" bind:value={row.alias} />
        <span class="arrow">→</span>
        <input placeholder="真实模型，如 glm-5.3-flash" spellcheck="false" bind:value={row.model} />
        <button class="btn ghost danger" onclick={() => form.removeAlias(row)} aria-label="删除">✕</button>
      </div>
    {/each}
    <button class="btn ghost" onclick={() => form.addAlias()}>+ 添加别名</button>
  </section>
{/if}

<style>
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
</style>
