// The history entry an open card holds, so Android's Back closes the card.
// Opening a card pushes one entry (another card, or an open Settings group's
// entry, is reused); dismissing the card with × or Escape pops it again, so
// Back is never needed more than once to leave the app.

export interface CardHistory {
  opened(sessionId: string): void;
  dismissed(): void;
}

export function makeCardHistory(h: History = window.history): CardHistory {
  const holding = () => typeof (h.state as { card?: unknown } | null)?.card === "string";
  // A Settings group's entry (settings-history.ts), as when a notification
  // is tapped with a group open, is taken over rather than stacked on.
  const settings = () => typeof (h.state as { settings?: unknown } | null)?.settings === "string";
  return {
    opened(sessionId) {
      if (holding() || settings()) h.replaceState({ card: sessionId }, "");
      else h.pushState({ card: sessionId }, "");
    },
    dismissed() {
      // The popstate this causes closes nothing: the card is already shut.
      if (holding()) h.back();
    },
  };
}
