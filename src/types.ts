export type CardState = "awaiting" | "working" | "completed" | "idle";
export type AwaitKind = "question" | "plan" | "permission";

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
}

export const COLUMNS: { state: CardState; title: string }[] = [
  { state: "awaiting", title: "Awaiting Decision" },
  { state: "working", title: "Working" },
  { state: "completed", title: "Completed" },
  { state: "idle", title: "Idle" },
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
