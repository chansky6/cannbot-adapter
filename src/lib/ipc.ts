import { invoke } from "@tauri-apps/api/core";

export type VkPolicy = "auto" | "builtin_only" | "personal_only" | "manual";

export interface AppConfig {
  port: number;
  bind: string;
  api_key: string | null;
  /** Fully masked sentinel from the backend (never the plaintext VK). */
  user_vk: string | null;
  device_id: string | null;
  vk_policy: VkPolicy;
  manual_vk: string | null;
  model_aliases: Record<string, string>;
}

export interface JwtStatusView {
  ok: boolean;
  message: string;
}

export interface StatusView {
  server_running: boolean;
  server_addr: string | null;
  server_error: string | null;
  jwt: JwtStatusView;
  vk: { builtin: string | null; personal: string | null };
  vk_policy: VkPolicy;
  models_count: number | null;
  models_synced_at: number | null;
  alias_count: number;
  api_base: string;
}

export interface ModelInfo {
  model: string;
  alias_of: string | null;
  /** Alias configured for this model (frontend-editable). */
  alias: string | null;
  builtin: boolean;
  personal: boolean;
  context_length: number | null;
  max_tokens: number | null;
}

export interface UsageRow {
  date: string;
  model: string;
  requests: number;
  input_tokens: number;
  output_tokens: number;
}

export function getModels(): Promise<ModelInfo[]> {
  return invoke("get_models");
}

export function getConfig(): Promise<AppConfig> {
  return invoke("get_config");
}

export function saveConfig(config: AppConfig): Promise<{ restarted: boolean }> {
  return invoke("save_config", { newConfig: config });
}

export function regenerateApiKey(): Promise<string> {
  return invoke("regenerate_api_key");
}

export function getStatus(): Promise<StatusView> {
  return invoke("get_status");
}

export function getAutostart(): Promise<boolean> {
  return invoke("get_autostart");
}

export function setAutostart(enabled: boolean): Promise<void> {
  return invoke("set_autostart", { enabled });
}

/** Lightweight refresh of the gateway model list (no chat request). */
export function fetchModelsCmd(): Promise<{ models_count: number }> {
  return invoke("fetch_models_cmd");
}

export function getUsage(days?: number): Promise<UsageRow[]> {
  return invoke("get_usage", { days });
}

export function clearUsage(): Promise<void> {
  return invoke("clear_usage");
}
