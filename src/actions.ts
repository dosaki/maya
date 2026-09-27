export type CardAction =
  | { kind: "terminal" | "open"; sessionId: string }
  | { kind: "answer"; sessionId: string; questionIndex: number; optionIndex: number };

/**
 * What a click inside a card means: the Terminal button focuses the tab, an
 * option button answers a question, anything else opens the modal.
 */
export function cardActionFor(target: Element): CardAction | null {
  const cardEl = target.closest<HTMLElement>(".card");
  const sessionId = cardEl?.dataset.sessionId;
  if (!sessionId) return null;
  const actionEl = target.closest<HTMLElement>("[data-action]");
  const action = actionEl?.dataset.action;
  if (action === "terminal") return { kind: "terminal", sessionId };
  if (action === "answer" && actionEl) {
    return { kind: "answer", sessionId, questionIndex: Number(actionEl.dataset.q), optionIndex: Number(actionEl.dataset.opt) };
  }
  return { kind: "open", sessionId };
}
