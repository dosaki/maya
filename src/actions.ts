export type CardAction =
  | { kind: "terminal" | "open" | "pr" | "compact" | "close"; sessionId: string }
  | { kind: "answer"; sessionId: string; askId: number; questionIndex: number; optionIndex: number };

/**
 * What a click inside a card means: the Terminal button focuses the tab, the
 * PR button opens the pull request, the Close button ends the session, an
 * option button answers a question, anything else opens the modal.
 */
export function cardActionFor(target: Element): CardAction | null {
  const cardEl = target.closest<HTMLElement>(".card");
  const sessionId = cardEl?.dataset.sessionId;
  if (!sessionId) return null;
  const actionEl = target.closest<HTMLElement>("[data-action]");
  const action = actionEl?.dataset.action;
  if (action === "terminal") return { kind: "terminal", sessionId };
  if (action === "pr") return { kind: "pr", sessionId };
  if (action === "compact") return { kind: "compact", sessionId };
  if (action === "close") return { kind: "close", sessionId };
  if (action === "answer" && actionEl) {
    return {
      kind: "answer",
      sessionId,
      askId: Number(actionEl.dataset.ask),
      questionIndex: Number(actionEl.dataset.q),
      optionIndex: Number(actionEl.dataset.opt),
    };
  }
  return { kind: "open", sessionId };
}
