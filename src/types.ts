import { iconButton } from "./icons";

export type CardState = "awaiting" | "working" | "completed" | "idle";
/** "text" is a question asked in prose at the end of a turn: no picker, reply through the inbox. */
export type AwaitKind = "question" | "plan" | "permission" | "text";

export interface Choice {
  label: string;
  description: string;
}

export interface Question {
  question: string;
  header: string;
  options: Choice[];
  multiSelect: boolean;
}

/** The agent runner behind a session; "other" is one this build does not know. */
export type Harness = "claude-code" | "codex" | "antigravity" | "grok" | "kiro" | "opencode" | "other";

export type GitProvider = "github" | "gitlab";

export interface PullRequest {
  number: number;
  url: string;
  /** "open", "draft", "merged" or "closed". */
  state: string;
  /** Absent from an older Maya, which only knew GitHub. */
  forge?: GitProvider;
}

/** `#451` on GitHub, `!12` on GitLab. */
export function prRef(pr: { forge?: GitProvider; number: number }): string {
  return `${pr.forge === "gitlab" ? "!" : "#"}${pr.number}`;
}

export interface ContextUsage {
  used: number;
  window: number;
  /** 0 to 100. */
  percent: number;
}

/** From this percentage the board offers to compact the session. */
export const COMPACT_AT = 75;

export function contextBand(percent: number): "low" | "mid" | "high" {
  return percent >= COMPACT_AT ? "high" : percent >= 50 ? "mid" : "low";
}

/** "124k" or "1M": tokens rounded for a tooltip. */
export function formatTokens(n: number): string {
  if (n >= 1_000_000 && n % 1_000_000 === 0) return `${n / 1_000_000}M`;
  return `${Math.round(n / 1000)}k`;
}

/** A vertical meter along a card's edge, filled to the context percentage. */
export function contextMeter(ctx: ContextUsage): HTMLElement {
  const meter = document.createElement("div");
  meter.className = "card__meter";
  meter.dataset.band = contextBand(ctx.percent);
  meter.title = `Context ${ctx.percent}% · ${formatTokens(ctx.used)} of ${formatTokens(ctx.window)}`;
  const fill = document.createElement("div");
  fill.className = "card__meter-fill";
  fill.style.height = `${ctx.percent}%`;
  meter.append(fill);
  return meter;
}

export function compactButton(className = "card__btn"): HTMLButtonElement {
  const b = iconButton("compact", "Compact context", className);
  b.dataset.action = "compact";
  b.title = "Compact context: types /compact into the session's terminal";
  return b;
}

/** A pull request waiting on the user, from the Pull Requests tab. */
export interface ReviewPr {
  number: number;
  forge: GitProvider;
  /** The repo's full path (owner/name, or group/sub/name). */
  repo: string;
  title: string;
  author: string;
  url: string;
  isDraft: boolean;
  updatedAt: string;
  reasons: ("review" | "assigned")[];
  /** The user's own review; null when not reviewed yet, or their own PR. */
  myReview: ReviewMark | null;
  /** What other people (not bots, nor the author) decided on it; null before any review. */
  prReview: ReviewMark | null;
  /** Who gave `prReview`. */
  prReviewers: string[];
  /** The repo's clone addresses from the forge's API; the page does not use them. */
  sshUrl?: string | null;
  httpsUrl?: string | null;
}

export type ReviewMark = "approved" | "commented" | "changesRequested";

export interface ForgeError {
  forge: GitProvider;
  /** The whole line, e.g. "GitLab check failed: …". */
  message: string;
}

export interface ReviewState {
  prs: ReviewPr[];
  errors: ForgeError[];
  /** Neither GitHub nor GitLab is switched on. */
  off: boolean;
  fetchedAt: number | null;
}

export interface Card {
  sessionId: string;
  pid: number;
  name: string;
  cwd: string;
  state: CardState;
  stateSince: number;
  snippet: string;
  awaiting: { kind: AwaitKind; detail: string; questions: Question[] } | null;
  hasInbox: boolean;
  harness: Harness;
  /** The PR for the session directory's branch, once looked up. */
  pr: PullRequest | null;
  /** Context window usage after the last assistant turn. */
  context: ContextUsage | null;
  /** The assistant machine this card came from; absent or null for a local session. */
  machine?: string | null;
  /** That machine's IP address as the main sees it; absent for a local session. */
  machineAddress?: string | null;
  /** That machine's platform as it reported it (`macos`, `linux`, `windows`); absent for a local session. */
  machinePlatform?: string | null;
  /** The assistant's tmux session name for this card, if it has one. */
  terminal?: string | null;
  /** The model the session runs on, when its agent says (OpenCode's provider/model). */
  model?: string | null;
  /** True when the machine has not reported for a while or is disconnected. */
  stale?: boolean;
}

/** The PR button: "PR #12" ("MR !12" on GitLab), with the state and URL in its tooltip. */
export function prButton(pr: PullRequest, className = "card__btn"): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = className;
  b.dataset.action = "pr";
  b.textContent = `${pr.forge === "gitlab" ? "MR" : "PR"} ${prRef(pr)}`;
  b.title = `${pr.state} · ${pr.url}`;
  return b;
}

export const COLUMNS: { state: CardState; title: string }[] = [
  { state: "idle", title: "Idle" },
  { state: "working", title: "Working" },
  { state: "awaiting", title: "Awaiting Decision" },
  { state: "completed", title: "Completed" },
];

/** One line of an edit's diff; `line` is its number in the new file (the old one for a removal), when known. */
export interface DiffLine {
  op: "add" | "del" | "same";
  line?: number | null;
  text: string;
}

export interface Turn {
  kind: "user" | "assistant" | "tool" | "peer";
  /** A compaction or an interrupt, sent as a tool line so older Mayas can read it. */
  notice?: boolean;
  text: string;
  /** A tool call's result: at most 200 lines, 40 from another machine. */
  output?: string | null;
  /** Lines in the whole result before any cap; absent from an older Maya. */
  outputLines?: number;
  /** The tool call failed. */
  isError?: boolean;
  /** An edit's lines: at most 200, 40 from another machine. */
  diff?: DiffLine[] | null;
  /** Lines the whole edit adds and removes. */
  added?: number;
  removed?: number;
}

export const STATE_LABEL: Record<CardState, string> = {
  awaiting: "Awaiting Decision",
  working: "Working",
  completed: "Completed",
  idle: "Idle",
};

const PLATFORM_NAMES: Record<string, string> = { macos: "macOS", linux: "Linux", windows: "Windows" };

/**
 * The remote glyph's tooltip: "Runs on <label> (<platform>), <address>".
 * The platform is left out when unknown, and the address when the label
 * already carries it. When the card has a tmux session name, it appends
 * "; attach: tmux attach -t <terminal>" so the main can say how to reach it.
 */
/**
 * The toast for a remote card's Terminal action: the main cannot focus
 * another machine's terminal, so it says where the session runs and, when
 * the card has a tmux session name, how to attach to it.
 */
export function remoteTerminalToast(card: Pick<Card, "machine" | "terminal">): string {
  const base = `That session runs on ${card.machine ?? "another machine"}`;
  return card.terminal ? `${base}; attach: tmux attach -t ${card.terminal}` : base;
}

export function remoteTitle(machine: string, address?: string | null, platform?: string | null, terminal?: string | null): string {
  const shown = platform ? ` (${PLATFORM_NAMES[platform] ?? platform})` : "";
  const base = !address || machine.includes(`(${address})`) ? `Runs on ${machine}${shown}` : `Runs on ${machine}${shown}, ${address}`;
  return terminal ? `${base}; attach: tmux attach -t ${terminal}` : base;
}

/** A session that was running when its machine's Maya last stopped. */
export interface Remembered {
  agent: Harness;
  sessionId: string;
  dir: string;
  name: string;
  firstSeenMs: number;
  lastSeenMs: number;
  goneAtMs?: number | null;
  offered?: boolean;
  /** When it was first offered; carried for a day after. */
  offeredAtMs?: number | null;
  /** Why its last resume failed ("folder missing"…). */
  error?: string | null;
}

/** A banner row: `machine` is absent for this machine; `waiting` while that machine is away. */
export interface RememberedRow extends Remembered {
  machine?: string | null;
  waiting?: boolean;
}

/** What became of one row of a resume; `error` is its reason. */
export interface ResumeOutcome {
  machine?: string | null;
  agent: Harness;
  sessionId: string;
  name: string;
  error?: string | null;
}

/** How the session viewer draws a conversation. */
export type ViewerTheme = "chat" | "terminal";
