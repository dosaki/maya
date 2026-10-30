import "@xterm/xterm/css/xterm.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { cardActionFor } from "./actions";
import { answerGuard, stopGuard } from "./answer";
import { boardCards, countable, renderBoard, swapBoard } from "./board";
import { makeClickGuard } from "./clickguard";
import { openModal, refreshModal, setProgress } from "./modal";
import { closeNewSession, openNewSession } from "./newsession";
import { closeResume, openResume } from "./resume";
import { nextEnableDelay } from "./options";
import { makeProgress } from "./progress";
import { approvalCount, deploymentActionFor, renderDeployments, withoutRun, type RowUi } from "./deployments";
import { makeReviewsView, OPEN_SETTINGS_GROUP, reviewActionFor, waitingCount } from "./reviews";
import { filterCards, makeSessionSearch } from "./search";
import { initRemembered } from "./remembered";
import { makeTabs } from "./tabs";
import { initSettings } from "./settings";
import { showToast } from "./toast";
import { prRef, remoteTerminalToast, type Card, type DeploymentState, type ReviewState } from "./types";
import { initMute } from "./mute";
import { initVoice } from "./voice";
import { initChat } from "./chat";
import { initDebug } from "./debug";
import { maybeShowFirstRun } from "./firstrun";
import { maybeShowKeysNotice } from "./notice";
import { updates } from "./update";
import { askQuit } from "./quitguard";

let cards: Card[] = [];
const guard = makeClickGuard();
const progress = makeProgress();
setProgress(progress);
let retry: ReturnType<typeof setTimeout> | undefined;
let enableTimer: ReturnType<typeof setTimeout> | undefined;
let known = new Set<string>();
let pointer: { x: number; y: number } | null = null;
let reviews: ReviewState = { prs: [], errors: [], off: false, fetchedAt: null };
let deployments: DeploymentState = { runs: [], error: null, off: false, assistant: false, fetchedAt: null };
const deployUi: RowUi = { busy: new Set(), errors: new Map() };
let clonesDir: string | null = null;
let tabs: ReturnType<typeof makeTabs> | null = null;
let reviewsView: ReturnType<typeof makeReviewsView> | null = null;
let search: ReturnType<typeof makeSessionSearch> | null = null;

function paintReviews(): void {
  const host = document.getElementById("reviews");
  if (!host) return;
  reviewsView ??= makeReviewsView(host);
  reviewsView.update(reviews, Date.now(), { cards, clonesDir });
  tabs?.setCount(waitingCount(reviews.prs));
}

function paintDeployments(): void {
  const host = document.getElementById("deployments");
  if (!host) return;
  host.replaceChildren(renderDeployments(deployments, Date.now(), deployUi));
  tabs?.setDeploymentCount(approvalCount(deployments.runs));
}

async function deploymentAction(target: Element): Promise<void> {
  const action = deploymentActionFor(target);
  if (!action) return;
  if (action.kind === "git-settings") {
    document.dispatchEvent(new CustomEvent(OPEN_SETTINGS_GROUP, { detail: "git" }));
    tabs?.show("settings");
  } else if (action.kind === "open-run" || action.kind === "open-blocker") {
    try {
      await invoke("open_deployment", { repo: action.repo, runId: action.runId, blocker: action.kind === "open-blocker" });
    } catch (e) {
      showToast(String(e));
    }
  } else {
    const { repo, runId, environmentIds } = action;
    if (deployUi.busy.has(runId)) return;
    deployUi.busy.add(runId);
    deployUi.errors.delete(runId);
    paintDeployments();
    try {
      await invoke(action.kind === "approve" ? "deployment_approve" : "deployment_reject", { repo, runId, environmentIds });
      deployments = withoutRun(deployments, repo, runId);
    } catch (e) {
      deployUi.errors.set(runId, String(e));
      // The backend may have dropped the row already; the toast says why.
      if (!deployments.runs.some((r) => r.repo === repo && r.runId === runId)) showToast(String(e));
    }
    deployUi.busy.delete(runId);
    paintDeployments();
  }
}

async function reviewAction(target: Element): Promise<void> {
  const action = reviewActionFor(target);
  if (!action) return;
  try {
    if (action.kind === "git-settings") {
      document.dispatchEvent(new CustomEvent(OPEN_SETTINGS_GROUP, { detail: "git" }));
      tabs?.show("settings");
    } else if (action.kind === "terminal") {
      await focus(action.pid);
    } else if (action.kind === "open-pr") {
      await invoke("open_review_pr", { forge: action.forge, repo: action.repo, number: action.number });
    } else {
      const dir = await invoke<string>("review_pr", { forge: action.forge, repo: action.repo, number: action.number });
      showToast(`Reviewing ${prRef(action)} in ${dir.split("/").filter(Boolean).pop() ?? dir}`);
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
  const query = search?.query() ?? "";
  const shown = filterCards(boardCards(cards, debugMode), query);
  swapBoard(host, renderBoard(shown, Date.now(), (c) => progress.next(c), query.trim() ? "No matches" : undefined));
  const counted = countable(cards);
  search?.setCount(countable(shown).length, counted.length);
  guard.markPaint(Date.now(), { movedUnderPointer: targetUnderPointer() !== before });
  // Option buttons rendered inside the open delay: repaint once it has elapsed.
  const delay = nextEnableDelay(cards, Date.now());
  if (enableTimer) clearTimeout(enableTimer);
  enableTimer = delay === null ? undefined : setTimeout(() => { enableTimer = undefined; paint(); }, delay + 50);
  const counts = { idle: 0, working: 0, awaiting: 0, completed: 0 };
  for (const c of counted) counts[c.state] += 1;
  tabs?.setSessionCounts(counts);
}

/**
 * Opens the viewer of the local session `pid` when it runs in a terminal
 * Maya hosts: that terminal is in the viewer, not a window. False otherwise.
 */
function openNative(pid: number): boolean {
  const card = cards.find((c) => c.pid === pid && !c.machine && c.native);
  if (!card) return false;
  closeNewSession();
  void openModal(card);
  return true;
}

/** Brings forward the terminal of the local session `pid`. */
async function focus(pid: number): Promise<void> {
  if (openNative(pid)) return;
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

async function closeSession(card: Card): Promise<void> {
  try {
    await invoke("close_session", { sessionId: card.sessionId });
    showToast(`Closed ${card.name}`);
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
  else if (action.kind === "close") void closeSession(card);
  else if (action.kind === "stop") void stop(card, target);
  else if (action.kind === "answer") void answer(card, action.questionIndex, action.optionIndex, target);
  else {
    closeNewSession();
    void openModal(card);
  }
}

/** Stop from a card; the button stays disabled until the card moves on, or comes back on an error. */
async function stop(card: Card, target: Element): Promise<void> {
  const button = target.closest<HTMLButtonElement>("button[data-action=stop]");
  if (!button) return;
  try {
    const outcome = await stopGuard.stop(card, button);
    if (outcome === "sent") showToast(`Sent Stop to ${card.name}`);
  } catch (e) {
    showToast(String(e));
    paint();
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

/** Debug mode, as last read from the config; the board filter (Maya's own card) reads it. */
let debugMode = false;

/** Show or hide the Debug tab; hiding it while it is open returns to Sessions. */
export function applyDebugMode(on: boolean): void {
  debugMode = on;
  for (const btn of document.querySelectorAll<HTMLElement>("[data-tab=debug]")) btn.hidden = !on;
  if (!on && tabs?.current() === "debug") tabs.show("sessions");
  paint();
}

export const isDebugMode = (): boolean => debugMode;

async function loadDebugMode(): Promise<void> {
  try {
    const cfg = await invoke<{ debugMode?: boolean }>("get_config");
    applyDebugMode(cfg.debugMode === true);
  } catch {
    /* no config yet: the Debug tab stays as the page has it */
  }
}

async function start(): Promise<void> {
  void initSettings();
  void updates.init();
  void initMute();
  void initVoice();
  initChat();
  void initDebug();
  void maybeShowFirstRun();
  void maybeShowKeysNotice(() => {
    document.dispatchEvent(new CustomEvent(OPEN_SETTINGS_GROUP, { detail: "keyboard" }));
    tabs?.show("settings");
  });
  const searchHost = document.getElementById("session-search");
  if (searchHost) search = makeSessionSearch(searchHost, () => paint());
  const rememberedHost = document.getElementById("remembered");
  if (rememberedHost) initRemembered(rememberedHost);
  // The search box and the banner belong to Sessions but sit outside the board pane.
  tabs = makeTabs((tab) => {
    if (searchHost) searchHost.hidden = tab !== "sessions";
    if (rememberedHost) rememberedHost.hidden = tab !== "sessions";
  });
  void loadDebugMode();
  window.addEventListener("maya:config-changed", () => void loadDebugMode());
  // Cmd+F (Ctrl+F) finds a session, unless a dialog is open over the board.
  window.addEventListener("keydown", (ev) => {
    if (ev.key.toLowerCase() === "f" && (ev.metaKey || ev.ctrlKey) && !ev.shiftKey && !ev.altKey && tabs?.current() === "sessions" && !document.getElementById("modal-host")?.childElementCount) {
      ev.preventDefault();
      search?.focus();
    }
  });
  document.getElementById("reviews")?.addEventListener("click", (ev) => void reviewAction(ev.target as Element));
  await listen<ReviewState>("reviews", (e) => {
    reviews = e.payload;
    paintReviews();
  });
  paintReviews();
  document.getElementById("deployments")?.addEventListener("click", (ev) => void deploymentAction(ev.target as Element));
  await listen<DeploymentState>("deployments", (e) => {
    deployments = e.payload;
    paintDeployments();
  });
  paintDeployments();
  void invoke<DeploymentState>("list_deployments").then((d) => {
    deployments = d;
    paintDeployments();
  }).catch((e) => showToast(String(e)));
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

  // A quit held because it would stop a working Native session.
  await listen<{ names: string[] }>("quit-confirm", (e) => {
    void askQuit(e.payload.names, false).then((yes) => (yes ? invoke("quit_confirmed") : undefined));
  });
  // "Show me …" by voice for a session in a terminal Maya hosts.
  await listen<{ pid: number }>("open-session", (e) => void openNative(e.payload.pid));
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
  setInterval(() => { paint(); paintReviews(); paintDeployments(); }, 10_000); // refresh the age labels
}

void start();
