export interface CardAction {
  kind: "terminal" | "open";
  sessionId: string;
}

/** What a click inside a card means: the Terminal button focuses the tab; anything else opens the modal. */
export function cardActionFor(target: Element): CardAction | null {
  const cardEl = target.closest<HTMLElement>(".card");
  const sessionId = cardEl?.dataset.sessionId;
  if (!sessionId) return null;
  const action = target.closest<HTMLElement>("[data-action]")?.dataset.action;
  return { kind: action === "terminal" ? "terminal" : "open", sessionId };
}
