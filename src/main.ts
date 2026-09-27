import { listen } from "@tauri-apps/api/event";

const pre = document.createElement("pre");
pre.style.cssText = "font: 12px Menlo, monospace; white-space: pre-wrap; padding: 12px;";
pre.textContent = "waiting for sessions event…";
document.body.replaceChildren(pre);

void listen("sessions", (e) => {
  const cards = e.payload as Array<Record<string, unknown>>;
  pre.textContent = `sessions event: ${cards.length} cards @ ${new Date().toISOString()}\n\n` + JSON.stringify(cards, null, 1).slice(0, 6000);
});
