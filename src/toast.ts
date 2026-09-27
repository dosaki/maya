let timer: ReturnType<typeof setTimeout> | undefined;

export function showToast(message: string, ms = 3000): void {
  const el = document.getElementById("toast");
  if (!el) return;
  el.textContent = message;
  el.hidden = false;
  if (timer) clearTimeout(timer);
  timer = setTimeout(() => {
    el.hidden = true;
  }, ms);
}
