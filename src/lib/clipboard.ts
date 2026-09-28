import { showToast } from "./toast.svelte";

export async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    showToast("ok", "已复制到剪贴板");
  } catch {
    showToast("err", "复制失败");
  }
}
