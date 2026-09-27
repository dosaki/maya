import type { Card } from "./types";

/**
 * Which question of an ask comes next, per session. An entry is only valid
 * for the awaiting timestamp it was recorded against, so a new question set
 * starts again from the first question.
 */
export function makeProgress() {
  const state = new Map<string, { since: number; next: number }>();
  const entry = (card: Card) => {
    const e = state.get(card.sessionId);
    return e && e.since === card.stateSince ? e : { since: card.stateSince, next: 0 };
  };
  return {
    next: (card: Card) => entry(card).next,
    advance: (card: Card) => {
      const e = entry(card);
      state.set(card.sessionId, { since: e.since, next: e.next + 1 });
    },
    reset: (sessionId: string) => {
      state.delete(sessionId);
    },
  };
}

export type Progress = ReturnType<typeof makeProgress>;
