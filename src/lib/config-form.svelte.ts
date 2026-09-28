import type { AppConfig, VkPolicy } from "./ipc";
import { showToast } from "./toast.svelte";

/// Masked placeholder the backend sends instead of the stored VK.
export const VK_MASK_SENTINEL = "**********";

export interface AliasRow {
  alias: string;
  model: string;
}

/**
 * Reactive holder for every editable config field, plus the VK masking
 * round-trip rules: an untouched (sentinel) field means "keep stored value",
 * an edited-but-emptied field means "clear it".
 */
export class ConfigForm {
  port = $state(8266);
  bind = $state("127.0.0.1");
  apiKey = $state("");
  showKey = $state(false);
  userVk = $state("");
  showVk = $state(false);
  vkEdited = $state(false);
  vkPolicy = $state<VkPolicy>("auto");
  manualVk = $state("");
  manualVkEdited = $state(false);
  aliasRows = $state<AliasRow[]>([]);

  // device_id is managed by the backend (stable per install); keep the
  // original value so saves don't clobber it.
  deviceId: string | null = null;

  apply(cfg: AppConfig) {
    this.port = cfg.port;
    this.bind = cfg.bind === "0.0.0.0" ? "0.0.0.0" : "127.0.0.1";
    this.apiKey = cfg.api_key ?? "";
    this.userVk = cfg.user_vk ?? "";
    this.vkEdited = false;
    this.showVk = false;
    this.deviceId = cfg.device_id ?? null;
    this.vkPolicy = cfg.vk_policy ?? "auto";
    this.manualVk = cfg.manual_vk ?? "";
    this.manualVkEdited = false;
    this.aliasRows = Object.entries(cfg.model_aliases).map(([alias, model]) => ({ alias, model }));
  }

  build(): AppConfig {
    const aliases: Record<string, string> = {};
    for (const row of this.aliasRows) {
      const a = row.alias.trim();
      const m = row.model.trim();
      if (a && m) aliases[a] = m;
    }
    // The backend never sends the plaintext VK; only forward a new value the
    // user actually typed. An untouched (sentinel) field means "keep stored".
    const userVkOut = this.vkEdited ? (this.userVk.trim() === "" ? null : this.userVk.trim()) : VK_MASK_SENTINEL;
    const manualVkOut = this.manualVkEdited
      ? this.manualVk.trim() === ""
        ? null
        : this.manualVk.trim()
      : this.manualVk
        ? VK_MASK_SENTINEL
        : null;
    return {
      port: Number(this.port) || 0,
      bind: this.bind,
      api_key: this.apiKey.trim() === "" ? null : this.apiKey.trim(),
      user_vk: userVkOut,
      device_id: this.deviceId,
      vk_policy: this.vkPolicy,
      manual_vk: manualVkOut,
      model_aliases: aliases,
    };
  }

  /// Stable string fingerprint used for dirty checking.
  serialize(): string {
    const c = this.build();
    const aliases = Object.keys(c.model_aliases)
      .sort()
      .map((k) => `${k}=${c.model_aliases[k]}`)
      .join(";");
    return JSON.stringify([c.port, c.bind, c.api_key, c.user_vk, c.vk_policy, c.manual_vk, aliases]);
  }

  addAlias() {
    this.aliasRows.push({ alias: "", model: "" });
  }

  removeAlias(row: AliasRow) {
    const i = this.aliasRows.indexOf(row);
    if (i >= 0) this.aliasRows.splice(i, 1);
  }

  /// Set the alias for a real model row (fused alias editing in the model list).
  setModelAlias(model: string, alias: string) {
    const trimmed = alias.trim();
    if (!trimmed) {
      // Clearing: drop the alias bound to this model.
      this.aliasRows = this.aliasRows.filter((r) => r.model !== model);
      return;
    }
    // Conflict: the alias already points at a different model.
    const conflict = this.aliasRows.find((r) => r.alias === trimmed && r.model !== model);
    if (conflict) {
      showToast("err", `别名「${trimmed}」已被模型 ${conflict.model} 占用，请先删除原有映射`);
      return;
    }
    // Drop any previous alias pointing at this model, then set the new one.
    this.aliasRows = this.aliasRows.filter((r) => r.model !== model);
    this.aliasRows.push({ alias: trimmed, model });
  }

  modelAlias(model: string): string {
    return this.aliasRows.find((r) => r.model === model)?.alias ?? "";
  }

  /// Alias rows whose target model is not in the synced model list.
  orphanRows(models: string[]): AliasRow[] {
    return this.aliasRows.filter((r) => !models.includes(r.model));
  }
}
