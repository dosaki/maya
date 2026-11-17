export type CardAction =
  | { kind: "terminal" | "open" | "pr" | "compact" | "close" | "stop"; sessionId: string }
  | { kind: "answer" | "tick"; sessionId: string; askId: number; questionIndex: number; optionIndex: number }
  | { kind: "send-ticks"; sessionId: string; askId: number; questionIndex: number };

/**
 * What a click inside a card means: the Terminal button focuses the tab, the
 * PR button opens the pull request, the Close button ends the session, Stop
 * interrupts its turn, an option button answers a question (or, on a
 * multi-select one, ticks the option, and Send answers with the ticked
 * ones), anything else opens the modal.
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
  if (action === "stop") return { kind: "stop", sessionId };
  if (action === "send-ticks" && actionEl) {
    return { kind: "send-ticks", sessionId, askId: Number(actionEl.dataset.ask), questionIndex: Number(actionEl.dataset.q) };
  }
  if ((action === "answer" || action === "tick") && actionEl) {
    return {
      kind: action,
      sessionId,
      askId: Number(actionEl.dataset.ask),
      questionIndex: Number(actionEl.dataset.q),
      optionIndex: Number(actionEl.dataset.opt),
    };
  }
  return { kind: "open", sessionId };
}
