import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { renderBoard } from "./board";
import { openModal, refreshModal } from "./modal";
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
  const board = document.getElementById("board");
  board?.addEventListener("click", (ev) => {
    const target = ev.target as HTMLElement;
    const cardEl = target.closest<HTMLElement>(".card");
    if (!cardEl) return;
    const card = cards.find((c) => c.sessionId === cardEl.dataset.sessionId);
    if (!card) return;
    const action = target.closest<HTMLElement>("[data-action]")?.dataset.action;
    if (action === "terminal") {
      void focus(card.pid);
      return;
    }
    void openModal(card);
  });
  board?.addEventListener("keydown", (ev) => {
    if (ev.key !== "Enter") return;
    const cardEl = (ev.target as HTMLElement).closest<HTMLElement>(".card");
    const card = cards.find((c) => c.sessionId === cardEl?.dataset.sessionId);
    if (card) void openModal(card);
  });

  await listen<Card[]>("sessions", (e) => {
    cards = e.payload;
    paint();
    refreshModal(cards);
  });
  cards = await invoke<Card[]>("list_sessions");
  paint();
  setInterval(paint, 10_000); // refresh the age labels
}

void start();
