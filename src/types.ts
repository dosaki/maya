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
export type Harness = "claude-code" | "codex" | "antigravity";

export interface PullRequest {
  number: number;
  url: string;
  /** "open", "draft", "merged" or "closed". */
  state: string;
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
