import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { renderBoard } from "./board";
import { initSettings } from "./settings";
import { showToast } from "./toast";
import type { Card } from "./types";

let cards: Card[] = [];

function paint(): void {
  const host = document.getElementById("board");
  if (!host) return;
  host.replaceChildren(renderBoard(cards, Date.now()));
  const meta = document.getElementById("meta");
  if (meta) meta.textContent = `${cards.length} session${cards.length === 1 ? "" : "s"}`;
}

async function focus(pid: number): Promise<void> {
  try {
    await invoke("focus_session", { pid });
  } catch (e) {
    showToast(String(e));
  }
}

async function start(): Promise<void> {
  void initSettings();
  document.getElementById("board")?.addEventListener("click", (ev) => {
    const card = (ev.target as HTMLElement).closest<HTMLElement>(".card");
    if (card?.dataset.pid) void focus(Number(card.dataset.pid));
  });
  document.getElementById("board")?.addEventListener("keydown", (ev) => {
    if (ev.key !== "Enter") return;
    const card = (ev.target as HTMLElement).closest<HTMLElement>(".card");
    if (card?.dataset.pid) void focus(Number(card.dataset.pid));
  });

  await listen<Card[]>("sessions", (e) => {
    cards = e.payload;
    paint();
  });
  cards = await invoke<Card[]>("list_sessions");
  paint();
  setInterval(paint, 10_000); // refresh the age labels
}

void start();
