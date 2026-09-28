type ToastState = { kind: "ok" | "err"; text: string } | null;

let toast = $state<ToastState>(null);
let timer: ReturnType<typeof setTimeout> | undefined;

export function showToast(kind: "ok" | "err", text: string) {
  toast = { kind, text };
  clearTimeout(timer);
  timer = setTimeout(() => (toast = null), 3200);
}

export function toastState(): ToastState {
  return toast;
}
