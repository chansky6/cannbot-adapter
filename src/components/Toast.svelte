<script lang="ts">
  import { toastState } from "../lib/toast.svelte";

  const toast = $derived(toastState());
</script>

{#if toast}
  <div class="toast {toast.kind}">{toast.text}</div>
{/if}

<style>
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
