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

/** The agent runner behind a session. Only Claude Code has a source today. */
export type Harness = "claude-code" | "codex" | "antigravity" | "grok";

export interface PullRequest {
  number: number;
  url: string;
  /** "open", "draft", "merged" or "closed". */
  state: string;
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
  /** owner/name */
  repo: string;
  title: string;
  author: string;
  url: string;
  isDraft: boolean;
  updatedAt: string;
  reasons: ("review" | "assigned")[];
}

export interface ReviewState {
  prs: ReviewPr[];
  error: string | null;
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
  /** True when the machine has not reported for a while or is disconnected. */
  stale?: boolean;
}

/** The PR button: "PR #12", with the state and URL in its tooltip. */
export function prButton(pr: PullRequest, className = "card__btn"): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = className;
  b.dataset.action = "pr";
  b.textContent = `PR #${pr.number}`;
  b.title = `${pr.state} · ${pr.url}`;
  return b;
}

export const COLUMNS: { state: CardState; title: string }[] = [
  { state: "idle", title: "Idle" },
  { state: "working", title: "Working" },
  { state: "awaiting", title: "Awaiting Decision" },
  { state: "completed", title: "Completed" },
];

export interface Turn {
  kind: "user" | "assistant" | "tool" | "peer";
  text: string;
}

export const STATE_LABEL: Record<CardState, string> = {
  awaiting: "Awaiting Decision",
  working: "Working",
  completed: "Completed",
  idle: "Idle",
};
