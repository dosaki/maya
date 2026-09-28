import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { cardActionFor } from "./actions";
import { answerGuard } from "./answer";
import { renderBoard, swapBoard } from "./board";
import { makeClickGuard } from "./clickguard";
import { openModal, refreshModal, setProgress } from "./modal";
import { closeNewSession, openNewSession } from "./newsession";
import { nextEnableDelay } from "./options";
import { makeProgress } from "./progress";
import { initSettings } from "./settings";
import { showToast } from "./toast";
import type { Card } from "./types";

let cards: Card[] = [];
const guard = makeClickGuard();
const progress = makeProgress();
setProgress(progress);
let retry: ReturnType<typeof setTimeout> | undefined;
let enableTimer: ReturnType<typeof setTimeout> | undefined;
let known = new Set<string>();
let pointer: { x: number; y: number } | null = null;

/** Which card and button sit under the pointer, as a comparable key. */
function targetUnderPointer(): string {
  if (!pointer) return "";
  const el = document.elementFromPoint(pointer.x, pointer.y);
  const card = el?.closest<HTMLElement>(".card");
  const action = el?.closest<HTMLElement>("[data-action]");
  return `${card?.dataset.sessionId ?? ""}:${action?.dataset.action ?? ""}`;
}

function paint(): void {
  if (!guard.canPaint(Date.now())) {
    // The pointer is busy over the board; try again shortly rather than
    // moving cards under it.
    if (!retry) retry = setTimeout(() => { retry = undefined; paint(); }, 200);
    return;
  }
  const host = document.getElementById("board");
  if (!host) return;
  const before = targetUnderPointer();
  swapBoard(host, renderBoard(cards, Date.now(), (c) => progress.next(c)));
  guard.markPaint(Date.now(), { movedUnderPointer: targetUnderPointer() !== before });
  // Option buttons rendered inside the open delay: repaint once it has elapsed.
  const delay = nextEnableDelay(cards, Date.now());
  if (enableTimer) clearTimeout(enableTimer);
  enableTimer = delay === null ? undefined : setTimeout(() => { enableTimer = undefined; paint(); }, delay + 50);
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

async function compact(sessionId: string): Promise<void> {
  try {
    await invoke("compact_session", { sessionId });
    showToast("Sent /compact to the terminal");
  } catch (e) {
    showToast(String(e));
  }
}

async function openPr(sessionId: string): Promise<void> {
  try {
    await invoke("open_pr", { sessionId });
  } catch (e) {
    showToast(String(e));
  }
}

function act(target: Element): void {
  const action = cardActionFor(target);
  if (!action) return;
  const card = cards.find((c) => c.sessionId === action.sessionId);
  if (!card) return;
  if (action.kind === "terminal") void focus(card.pid);
  else if (action.kind === "pr") void openPr(card.sessionId);
  else if (action.kind === "compact") void compact(card.sessionId);
  else if (action.kind === "answer") void answer(card, action.questionIndex, action.optionIndex, target);
  else {
    closeNewSession();
    void openModal(card);
  }
}

async function answer(card: Card, questionIndex: number, optionIndex: number, button: Element): Promise<void> {
  try {
    const outcome = await answerGuard.answer(card, questionIndex, optionIndex, button as HTMLElement);
    if (outcome === "dropped") return;
    progress.advance(card);
    paint();
  } catch (e) {
    showToast(String(e));
  }
}

async function start(): Promise<void> {
  void initSettings();
  const board = document.getElementById("board");
  board?.addEventListener("pointerdown", () => guard.setPointerDown(true));
  window.addEventListener("pointerup", () => guard.setPointerDown(false));
  board?.addEventListener("pointermove", (ev) => {
    pointer = { x: ev.clientX, y: ev.clientY };
    guard.markPointerMove(Date.now());
  });
  board?.addEventListener("pointerleave", () => {
    pointer = null;
    guard.markPointerLeave();
  });
  board?.addEventListener("click", (ev) => {
    const target = ev.target as Element;
    if (target.closest("[data-action=new-session]")) {
      void openNewSession();
      return;
    }
    // The board may have just repainted under the cursor; ignore the click
    // rather than act on whichever card moved into place.
    if (!guard.allowClick(Date.now())) return;
    act(target);
  });
  board?.addEventListener("keydown", (ev) => {
    // Only the card itself: buttons already turn Enter into a click.
    const target = ev.target as Element;
    if (ev.key === "Enter" && target.classList.contains("card")) act(target);
  });

  await listen<Card[]>("sessions", (e) => {
    cards = e.payload;
    for (const c of cards) if (c.state !== "awaiting") progress.reset(c.sessionId);
    for (const id of known) if (!cards.some((c) => c.sessionId === id)) progress.reset(id);
    known = new Set(cards.map((c) => c.sessionId));
    paint();
    refreshModal(cards);
  });
  cards = await invoke<Card[]>("list_sessions");
  paint();
  setInterval(paint, 10_000); // refresh the age labels
}

void start();
