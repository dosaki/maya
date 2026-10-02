import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { cardActionFor } from "./actions";
import { answerGuard } from "./answer";
import { renderBoard, swapBoard } from "./board";
import { makeClickGuard } from "./clickguard";
import { openModal, refreshModal, setProgress } from "./modal";
import { closeNewSession, openNewSession } from "./newsession";
import { closeResume, openResume } from "./resume";
import { nextEnableDelay } from "./options";
import { makeProgress } from "./progress";
import { renderReviews, reviewActionFor } from "./reviews";
import { makeTabs } from "./tabs";
import { initSettings } from "./settings";
import { showToast } from "./toast";
import { remoteTerminalToast, type Card, type ReviewState } from "./types";
import { initMute } from "./mute";
import { initVoice } from "./voice";
import { initDebug } from "./debug";

let cards: Card[] = [];
const guard = makeClickGuard();
const progress = makeProgress();
setProgress(progress);
let retry: ReturnType<typeof setTimeout> | undefined;
let enableTimer: ReturnType<typeof setTimeout> | undefined;
let known = new Set<string>();
let pointer: { x: number; y: number } | null = null;
let reviews: ReviewState = { prs: [], error: null, fetchedAt: null };
let clonesDir: string | null = null;
let tabs: ReturnType<typeof makeTabs> | null = null;

function paintReviews(): void {
  const host = document.getElementById("reviews");
  if (!host) return;
  host.replaceChildren(renderReviews(reviews, Date.now(), { cards, clonesDir }));
  tabs?.setCount(reviews.prs.length);
}

async function reviewAction(target: Element): Promise<void> {
  const action = reviewActionFor(target);
  if (!action) return;
  try {
    if (action.kind === "terminal") {
      await focus(action.pid);
    } else if (action.kind === "open-pr") {
      await invoke("open_review_pr", { repo: action.repo, number: action.number });
    } else {
      const dir = await invoke<string>("review_pr", { repo: action.repo, number: action.number });
      showToast(`Reviewing #${action.number} in ${dir.split("/").filter(Boolean).pop() ?? dir}`);
    }
  } catch (e) {
    showToast(String(e));
  }
}

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
  const counts = { idle: 0, working: 0, awaiting: 0, completed: 0 };
  for (const c of cards) counts[c.state] += 1;
  tabs?.setSessionCounts(counts);
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
  if (action.kind === "terminal") {
    if (card.machine) showToast(remoteTerminalToast(card));
    else void focus(card.pid);
  } else if (action.kind === "pr") void openPr(card.sessionId);
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
  void initMute();
  void initVoice();
  void initDebug();
  tabs = makeTabs();
  document.getElementById("reviews")?.addEventListener("click", (ev) => void reviewAction(ev.target as Element));
  await listen<ReviewState>("reviews", (e) => {
    reviews = e.payload;
    paintReviews();
  });
  paintReviews();
  void invoke<ReviewState>("list_review_prs").then((r) => {
    reviews = r;
    paintReviews();
  });
  void invoke<string>("clones_dir").then((d) => {
    clonesDir = d;
    paintReviews();
  }).catch(() => undefined);
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
      closeResume();
      void openNewSession();
      return;
    }
    if (target.closest("[data-action=resume-session]")) {
      closeNewSession();
      void openResume();
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
    paintReviews();
    refreshModal(cards);
  });
  cards = await invoke<Card[]>("list_sessions");
  paint();
  setInterval(() => { paint(); paintReviews(); }, 10_000); // refresh the age labels
}

void start();
