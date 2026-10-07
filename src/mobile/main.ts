// The phone's page: the desktop's board, cards, modal and dialogs over the
// commands the mobile crate serves, laid out one to four columns wide.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { cardActionFor } from "../actions";
import { answerGuard } from "../answer";
import { renderBoard, swapBoard } from "../board";
import { makeClickGuard } from "../clickguard";
import { initDebug } from "../debug";
import { setLocalMachine } from "../machines";
import { closeModal, openModal, refreshModal, setComposerOptions, setOnDismiss, setProgress } from "../modal";
import { closeNewSession, openNewSession } from "../newsession";
import { nextEnableDelay } from "../options";
import { makeProgress } from "../progress";
import { closeResume, openResume } from "../resume";
import { makeTabs } from "../tabs";
import { showToast } from "../toast";
import type { Card, CardState } from "../types";
import { makeCardHistory } from "./card-history";
import { initNetwork, type NetworkStatus } from "./network";
import { watchTaps } from "./notify-tap";
import { initSetup } from "./setup";
import { activeColumn, columnWidth, columnsFor, openingColumn, renderStrip, scrollLeftFor, visibleColumns } from "./strip";

setLocalMachine(false);
setComposerOptions({ attachButton: true });

let cards: Card[] = [];
let status: NetworkStatus | null = null;
let opened = false;
let pendingTap: string | null = null;
let colWidth = 0;
let columns = 1;
const guard = makeClickGuard();
const progress = makeProgress();
setProgress(progress);
let retry: ReturnType<typeof setTimeout> | undefined;
let enableTimer: ReturnType<typeof setTimeout> | undefined;
let known = new Set<string>();
let tabs: ReturnType<typeof makeTabs> | null = null;
/** "The server is stopped." with Start, shown in place of the columns while the server is down. */
let stopped: HTMLElement | null = null;
const cardHistory = makeCardHistory();
setOnDismiss(() => cardHistory.dismissed());

function boardEl(): HTMLElement | null {
  return document.querySelector<HTMLElement>("#board .board");
}

function counts(): Record<CardState, number> {
  const c = { idle: 0, working: 0, awaiting: 0, completed: 0 };
  for (const card of cards) c[card.state] += 1;
  return c;
}

/** Re-measures the columns for the viewport and keeps the active column in view. */
function layout(): void {
  const host = document.getElementById("board");
  if (!host) return;
  const width = host.clientWidth || window.innerWidth;
  const before = colWidth ? activeColumn(boardEl()?.scrollLeft ?? 0, colWidth) : null;
  columns = columnsFor(width);
  colWidth = columnWidth(width, columns);
  host.style.setProperty("--col-w", `${colWidth}px`);
  const board = boardEl();
  if (board && before !== null) board.scrollLeft = scrollLeftFor(before, colWidth);
  paintStrip();
}

function paintStrip(): void {
  const host = document.getElementById("strip-host");
  if (!host) return;
  const active = activeColumn(boardEl()?.scrollLeft ?? 0, colWidth);
  host.replaceChildren(renderStrip(counts(), visibleColumns(active, columns), (i) => boardEl()?.scrollTo({ left: scrollLeftFor(i, colWidth), behavior: "smooth" })));
}

function paintHint(host: HTMLElement): void {
  if (!status || status.assistants.length > 0) return;
  const hint = document.createElement("div");
  hint.className = "board__hint";
  const p = document.createElement("p");
  p.textContent = "Pair an assistant to see its sessions.";
  const open = document.createElement("button");
  open.type = "button";
  open.className = "card__btn card__btn--primary";
  open.textContent = "Open Network";
  open.addEventListener("click", () => tabs?.show("settings"));
  hint.append(p, open);
  host.prepend(hint);
}

function paint(): void {
  if (!guard.canPaint(Date.now())) {
    if (!retry) retry = setTimeout(() => { retry = undefined; paint(); }, 200);
    return;
  }
  const host = document.getElementById("board");
  if (!host) return;
  if (stopped) {
    if (host.firstElementChild !== stopped) host.replaceChildren(stopped);
    paintStrip();
    return;
  }
  swapBoard(host, renderBoard(cards, Date.now(), (c) => progress.next(c)));
  paintHint(host);
  guard.markPaint(Date.now(), { movedUnderPointer: false });
  const delay = nextEnableDelay(cards, Date.now());
  if (enableTimer) clearTimeout(enableTimer);
  enableTimer = delay === null ? undefined : setTimeout(() => { enableTimer = undefined; paint(); }, delay + 50);
  const board = boardEl();
  board?.addEventListener("scroll", paintStrip, { passive: true });
  if (!opened && board && cards.length > 0) {
    opened = true;
    board.scrollLeft = scrollLeftFor(openingColumn(cards), colWidth);
  }
  paintStrip();
}

async function run(label: string, f: () => Promise<unknown>, done?: string): Promise<void> {
  try {
    await f();
    if (done) showToast(done);
  } catch (e) {
    showToast(`${label}: ${String(e)}`);
  }
}

function open(card: Card): void {
  closeNewSession();
  closeResume();
  cardHistory.opened(card.sessionId);
  void openModal(card);
}

function act(target: Element): void {
  const action = cardActionFor(target);
  if (!action) return;
  const card = cards.find((c) => c.sessionId === action.sessionId);
  if (!card) return;
  if (action.kind === "terminal") showToast(`That session runs on ${card.machine ?? "another machine"}`);
  else if (action.kind === "pr") void run("Pull request", () => invoke("open_pr", { sessionId: card.sessionId }));
  else if (action.kind === "compact") void run("Compact", () => invoke("compact_session", { sessionId: card.sessionId }), "Sent /compact to the terminal");
  else if (action.kind === "close") void run("Close", () => invoke("close_session", { sessionId: card.sessionId }), `Closed ${card.name}`);
  else if (action.kind === "answer") void answer(card, action.questionIndex, action.optionIndex, target);
  else open(card);
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

function openFromTap(sessionId: string): void {
  const card = cards.find((c) => c.sessionId === sessionId);
  if (card) {
    tabs?.show("sessions");
    open(card);
  } else {
    pendingTap = sessionId;
  }
}

async function start(): Promise<void> {
  void initDebug();
  // The column strip sits above the panes but belongs to the board: it goes when another pane shows.
  tabs = makeTabs((tab) => {
    const strip = document.getElementById("strip-host");
    if (strip) strip.hidden = tab !== "sessions";
  });
  const recheckNotifications = initNetwork((s) => {
    status = s;
    paint();
  });
  initSetup({
    onStopped: (view) => {
      stopped = view;
      paint();
    },
    onPermissionAsked: recheckNotifications,
  });
  layout();
  window.addEventListener("resize", layout);
  // Android's back button pops the history entry a card pushed: close the card.
  window.addEventListener("popstate", () => closeModal());
  const board = document.getElementById("board");
  board?.addEventListener("pointerdown", () => guard.setPointerDown(true));
  for (const ev of ["pointerup", "pointercancel", "touchend"] as const) window.addEventListener(ev, () => guard.setPointerDown(false));
  board?.addEventListener("click", (ev) => {
    const target = ev.target as Element;
    if (target.closest("[data-action=new-session]")) {
      closeResume();
      void openNewSession();
      return;
    }
    if (target.closest("[data-action=resume-session]")) {
      closeNewSession();
      void openResume();
      return;
    }
    if (!guard.allowClick(Date.now())) return;
    act(target);
  });
  await listen<Card[]>("sessions", (e) => {
    cards = e.payload;
    for (const c of cards) if (c.state !== "awaiting") progress.reset(c.sessionId);
    for (const id of known) if (!cards.some((c) => c.sessionId === id)) progress.reset(id);
    known = new Set(cards.map((c) => c.sessionId));
    paint();
    refreshModal(cards);
    if (pendingTap) {
      const id = pendingTap;
      pendingTap = null;
      openFromTap(id);
    }
  });
  cards = await invoke<Card[]>("list_sessions");
  paint();
  void watchTaps(openFromTap);
  setInterval(paint, 10_000);
}

void start();
