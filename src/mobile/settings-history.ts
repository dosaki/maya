// The history entry an open Settings group holds, so Android's Back returns
// to the list of groups. Opening a group pushes one entry (another group
// opened over it reuses it); leaving the group by its back arrow, or the
// tab, pops it again, so Back is never needed more than once to leave the app.

export interface GroupHistory {
  opened(group: string): void;
  closed(): void;
  holding(): boolean;
}

export function makeGroupHistory(h: History = window.history): GroupHistory {
  const holding = () => typeof (h.state as { settings?: unknown } | null)?.settings === "string";
  return {
    opened(group) {
      if (holding()) h.replaceState({ settings: group }, "");
      else h.pushState({ settings: group }, "");
    },
    closed() {
      // The popstate this causes closes nothing: the group is already shut.
      if (holding()) h.back();
    },
    holding,
  };
}
