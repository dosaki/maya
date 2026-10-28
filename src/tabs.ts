import { STATE_LABEL, type CardState } from "./types";

export type Tab = "sessions" | "reviews" | "deployments" | "debug" | "settings" | "chat";

const KEY = "maya.tab";
const PANES: Record<Tab, string> = { sessions: "board", reviews: "reviews", deployments: "deployments", debug: "debug", settings: "settings", chat: "chat" };

function asTab(v: string | null | undefined): Tab {
  return v === "reviews" || v === "deployments" || v === "debug" || v === "settings" || v === "chat" ? v : "sessions";
}

function stored(): Tab {
  try {
    return asTab(localStorage.getItem(KEY));
  } catch {
    return "sessions";
  }
}

/**
 * Six panes, one visible: Sessions, Pull Requests, Deployments and Chat on the left, Debug and
 * Settings on the right. Sessions is the default; the choice is remembered
 * per browser. Switching only toggles `hidden`, so the board keeps its
 * scroll positions and the session watcher keeps running underneath.
 * `onChange` hears every switch, the first included, for what lives outside
 * the panes but belongs to one (the phone's column strip).
 */
export function makeTabs(onChange?: (tab: Tab) => void) {
  // A remembered tab whose button is hidden (Debug with debug mode off) is not a place to land.
  const remembered = stored();
  let current: Tab = document.querySelector<HTMLElement>(`[data-tab=${remembered}]`)?.hidden ? "sessions" : remembered;
  const show = (tab: Tab) => {
    current = tab;
    for (const [name, id] of Object.entries(PANES) as [Tab, string][]) {
      const pane = document.getElementById(id);
      if (pane) pane.hidden = name !== tab;
    }
    for (const btn of document.querySelectorAll<HTMLElement>("[data-tab]")) {
      btn.classList.toggle("tab--active", btn.dataset.tab === tab);
      btn.setAttribute("aria-selected", String(btn.dataset.tab === tab));
    }
    try {
      localStorage.setItem(KEY, tab);
    } catch {
      /* private mode: nothing to remember */
    }
    onChange?.(tab);
    window.dispatchEvent(new CustomEvent("maya:tab", { detail: tab }));
  };
  for (const btn of document.querySelectorAll<HTMLElement>("[data-tab]")) {
    btn.addEventListener("click", () => show(asTab(btn.dataset.tab)));
  }
  show(current);
  return {
    current: () => current,
    show,
    setCount: (n: number) => {
      const el = document.getElementById("reviews-count");
      if (el) el.textContent = n > 0 ? String(n) : "";
    },
    setDeploymentCount: (n: number) => {
      const el = document.getElementById("deployments-count");
      if (el) el.textContent = n > 0 ? String(n) : "";
    },
    /** One badge per column on the Sessions tab; zeros stay visible so the badges keep their places. */
    setSessionCounts: (counts: Record<CardState, number>) => {
      for (const [state, n] of Object.entries(counts) as [CardState, number][]) {
        const el = document.querySelector<HTMLElement>(`[data-count=${state}]`);
        if (!el) continue;
        el.textContent = String(n);
        el.title = `${n} ${STATE_LABEL[state].toLowerCase()}`;
      }
    },
  };
}
