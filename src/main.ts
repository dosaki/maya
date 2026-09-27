import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { cardActionFor } from "./actions";
import { renderBoard } from "./board";
import { makeClickGuard } from "./clickguard";
import { openModal, refreshModal, setProgress } from "./modal";
import { makeProgress } from "./progress";
import { initSettings } from "./settings";
import { showToast } from "./toast";
import type { Card } from "./types";

let cards: Card[] = [];
const guard = makeClickGuard();
const progress = makeProgress();
setProgress(progress);
let retry: ReturnType<typeof setTimeout> | undefined;

function paint(): void {
  if (!guard.canPaint(Date.now())) {
    // The pointer is busy over the board; try again shortly rather than
    // moving cards under it.
    if (!retry) retry = setTimeout(() => { retry = undefined; paint(); }, 200);
    return;
  }
  const host = document.getElementById("board");
  if (!host) return;
  host.replaceChildren(renderBoard(cards, Date.now(), (c) => progress.next(c)));
  guard.markPaint(Date.now());
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

function act(target: Element): void {
  const action = cardActionFor(target);
  if (!action) return;
  const card = cards.find((c) => c.sessionId === action.sessionId);
  if (!card) return;
  if (action.kind === "terminal") void focus(card.pid);
  else if (action.kind === "answer") void answer(card, action.questionIndex, action.optionIndex);
  else void openModal(card);
}

async function answer(card: Card, questionIndex: number, optionIndex: number): Promise<void> {
  try {
    await invoke("answer_question", { sessionId: card.sessionId, questionIndex, optionIndex });
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
  board?.addEventListener("pointermove", () => guard.markPointerMove(Date.now()));
  board?.addEventListener("pointerleave", () => guard.markPointerLeave());
  board?.addEventListener("click", (ev) => {
    // The board may have just repainted under the cursor; ignore the click
    // rather than act on whichever card moved into place.
    if (!guard.allowClick(Date.now())) return;
    act(ev.target as Element);
  });
  board?.addEventListener("keydown", (ev) => {
    // Only the card itself: buttons already turn Enter into a click.
    const target = ev.target as Element;
    if (ev.key === "Enter" && target.classList.contains("card")) act(target);
  });

  await listen<Card[]>("sessions", (e) => {
    cards = e.payload;
    for (const c of cards) if (c.state !== "awaiting") progress.reset(c.sessionId);
    paint();
    refreshModal(cards);
  });
  cards = await invoke<Card[]>("list_sessions");
  paint();
  setInterval(paint, 10_000); // refresh the age labels
}

void start();
