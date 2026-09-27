export type CardState = "awaiting" | "working" | "completed" | "idle";
export type AwaitKind = "question" | "plan" | "permission";

export interface Card {
  sessionId: string;
  pid: number;
  name: string;
  cwd: string;
  state: CardState;
  stateSince: number;
  snippet: string;
  awaiting: { kind: AwaitKind; detail: string } | null;
  hasInbox: boolean;
}

export const COLUMNS: { state: CardState; title: string }[] = [
  { state: "awaiting", title: "Awaiting Decision" },
  { state: "working", title: "Working" },
  { state: "completed", title: "Completed" },
  { state: "idle", title: "Idle" },
];
