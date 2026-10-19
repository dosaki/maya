// The Sessions banner: the sessions that were running when Maya last
// stopped, on this machine and its assistants, with Resume all, Choose…
// and Dismiss. The desktop and the phone both mount it.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { formatAge } from "./format";
import { harnessBadge } from "./harness";
import { showToast } from "./toast";
import type { RememberedRow, ResumeOutcome } from "./types";

/** One row's identity across machines: the same id may be on two. */
export function rowKey(r: Pick<RememberedRow, "machine" | "agent" | "sessionId">): string {
  return `${r.machine ?? ""}|${r.agent}|${r.sessionId}`;
}

/** `/Users/x/dev/calibre` as `~/dev/calibre`; Linux's `/home/x` and Windows' `C:\Users\x` too. */
export function tildePath(dir: string): string {
  return dir.replace(/^\/(?:Users|home)\/[^/]+(?=\/|$)/, "~").replace(/^[A-Za-z]:\\Users\\[^\\]+(?=\\|$)/, "~");
}

/** "5 sessions were running when Maya last stopped · 3 here · 2 on Gnowee". */
export function bannerText(rows: RememberedRow[]): string {
  const n = rows.length;
  const head = `${n} ${n === 1 ? "session was" : "sessions were"} running when Maya last stopped`;
  const groups = new Map<string, number>();
  for (const r of rows) {
    const label = r.machine ? `on ${r.machine}` : "here";
    groups.set(label, (groups.get(label) ?? 0) + 1);
  }
  if (groups.size < 2) return head;
  return `${head} · ${[...groups].map(([label, count]) => `${count} ${label}`).join(" · ")}`;
}

/** "Couldn't resume 1: raffinad-3c (folder missing)". */
export function failuresText(rows: RememberedRow[]): string {
  return `Couldn't resume ${rows.length}: ${rows.map((r) => `${r.name} (${r.error})`).join(", ")}`;
}

/** The toast after a resume: what started, what waits for its machine, what failed. */
export function resumeToast(outcomes: ResumeOutcome[]): string {
  const waiting = outcomes.filter((o) => o.error?.startsWith("waiting for "));
  const failed = outcomes.filter((o) => o.error && !o.error.startsWith("waiting for "));
  const ok = outcomes.length - waiting.length - failed.length;
  if (outcomes.length === 1 && failed.length === 1) return `Couldn't resume ${failed[0].name}: ${failed[0].error}`;
  const parts: string[] = [];
  if (ok > 0) parts.push(`Resuming ${ok}`);
  if (waiting.length > 0) parts.push(`skipped ${waiting.length} waiting for ${[...new Set(waiting.map((o) => o.machine ?? ""))].join(" and ")}`);
  if (failed.length > 0) parts.push(`${failed.length} couldn't be resumed`);
  if (parts.length === 0) return "Nothing to resume.";
  const text = parts.join("; ");
  return `${text.charAt(0).toUpperCase()}${text.slice(1)}.`;
}

export interface BannerModel {
  rows: RememberedRow[];
  /** The Choose list is open. */
  choosing: boolean;
  /** `rowKey`s ticked in the Choose list. */
  ticked: Set<string>;
  /** A resume or forget is running. */
  busy: boolean;
}

export interface BannerHandlers {
  onResumeAll(): void;
  onChoose(): void;
  onDismiss(): void;
  onDismissFailed(): void;
  onTick(key: string, on: boolean): void;
  onResumeChosen(): void;
  onCancel(): void;
  onForgetUnticked(): void;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = className;
  if (text !== undefined) e.textContent = text;
  return e;
}

function button(label: string, action: string, onClick: () => void, primary = false): HTMLButtonElement {
  const b = el("button", primary ? "card__btn card__btn--primary" : "card__btn", label);
  b.type = "button";
  b.dataset.action = action;
  b.addEventListener("click", onClick);
  return b;
}

/** The Choose list: grouped by machine, each row ticked unless its machine is away. */
function renderChoose(rows: RememberedRow[], m: BannerModel, h: BannerHandlers, nowMs: number): HTMLElement {
  const list = el("div", "remembered__choose");
  const groups = new Map<string, RememberedRow[]>();
  for (const r of rows) groups.set(r.machine ?? "", [...(groups.get(r.machine ?? "") ?? []), r]);
  for (const [machine, group] of groups) {
    list.append(el("h3", "remembered__machine", machine || "Here"));
    for (const r of group) {
      const key = rowKey(r);
      const line = el("label", "remembered__row");
      line.dataset.key = key;
      const box = el("input", "");
      box.type = "checkbox";
      box.checked = !r.waiting && m.ticked.has(key);
      box.disabled = !!r.waiting;
      box.addEventListener("change", () => h.onTick(key, box.checked));
      line.append(box, el("span", "remembered__name", r.name), el("span", "remembered__dir", tildePath(r.dir)), el("span", "remembered__age", r.waiting ? `waiting for ${r.machine}` : `${formatAge(r.lastSeenMs, nowMs)} ago`), harnessBadge(r.agent, "remembered__agent"));
      list.append(line);
    }
  }
  const n = rows.filter((r) => !r.waiting && m.ticked.has(rowKey(r))).length;
  const actions = el("div", "remembered__actions");
  const resume = button(`Resume ${n}`, "resume-chosen", h.onResumeChosen, true);
  resume.disabled = n === 0;
  actions.append(resume, button("Cancel", "cancel", h.onCancel), button("Forget unticked", "forget-unticked", h.onForgetUnticked));
  list.append(actions);
  return list;
}

/** The banner, or null when nothing was running. */
export function renderBanner(m: BannerModel, h: BannerHandlers, nowMs: number): HTMLElement | null {
  const pending = m.rows.filter((r) => !r.error);
  const failed = m.rows.filter((r) => r.error);
  if (pending.length === 0 && failed.length === 0) return null;
  const root = el("section", "remembered");
  if (pending.length > 0) {
    const line = el("div", "remembered__line");
    line.append(el("span", "remembered__text", bannerText(pending)));
    if (!m.choosing) line.append(button("Resume all", "resume-all", h.onResumeAll, true), button("Choose…", "choose", h.onChoose), button("Dismiss", "dismiss", h.onDismiss));
    root.append(line);
    if (m.choosing) root.append(renderChoose(pending, m, h, nowMs));
  }
  if (failed.length > 0) {
    const line = el("div", "remembered__line remembered__line--failed");
    line.append(el("span", "remembered__text", failuresText(failed)), button("Dismiss", "dismiss-failed", h.onDismissFailed));
    root.append(line);
  }
  if (m.busy) for (const b of root.querySelectorAll("button")) b.disabled = true;
  return root;
}

/** Mounts the banner into `host` and keeps it current from the `remembered` event. */
export function initRemembered(host: HTMLElement): void {
  const m: BannerModel = { rows: [], choosing: false, ticked: new Set(), busy: false };
  let shown = "";
  const paint = () => {
    // A repaint replaces every element: what had keyboard focus (a row's
    // checkbox, or a button) gets it back on its fresh twin.
    const focused = document.activeElement instanceof HTMLElement && host.contains(document.activeElement) ? document.activeElement : null;
    const key = focused?.closest<HTMLElement>("[data-key]")?.dataset.key;
    const action = focused instanceof HTMLButtonElement ? focused.dataset.action : undefined;
    const banner = renderBanner(m, handlers, Date.now());
    host.replaceChildren(...(banner ? [banner] : []));
    if (!focused) return;
    const again =
      key !== undefined
        ? [...host.querySelectorAll<HTMLElement>("[data-key]")].find((e) => e.dataset.key === key)?.querySelector("input")
        : action !== undefined
          ? host.querySelector<HTMLButtonElement>(`button[data-action="${action}"]`)
          : null;
    again?.focus();
  };
  const set = (rows: RememberedRow[]) => {
    const text = JSON.stringify(rows);
    if (text === shown) return;
    shown = text;
    m.rows = rows;
    const keys = new Set(rows.map(rowKey));
    m.ticked = new Set([...m.ticked].filter((k) => keys.has(k)));
    if (m.choosing && !rows.some((r) => !r.error)) m.choosing = false;
    paint();
  };
  const act = async (f: () => Promise<void>) => {
    m.busy = true;
    paint();
    try {
      await f();
    } catch (e) {
      showToast(String(e));
    }
    m.busy = false;
    paint();
  };
  const resume = (rows: RememberedRow[]) =>
    act(async () => {
      const outcomes = await invoke<ResumeOutcome[]>("resume_remembered", { rows });
      m.choosing = false;
      showToast(resumeToast(outcomes ?? []));
    });
  const forget = (rows: RememberedRow[]) =>
    act(async () => {
      await invoke("forget_remembered", { rows });
      m.choosing = false;
    });
  const pending = () => m.rows.filter((r) => !r.error);
  const handlers: BannerHandlers = {
    onResumeAll: () => void resume(pending()),
    onChoose: () => {
      m.choosing = true;
      m.ticked = new Set(pending().filter((r) => !r.waiting).map(rowKey));
      paint();
    },
    onDismiss: () => void forget(pending()),
    onDismissFailed: () => void forget(m.rows.filter((r) => r.error)),
    onTick: (key, on) => {
      if (on) m.ticked.add(key);
      else m.ticked.delete(key);
      paint();
    },
    onResumeChosen: () => void resume(pending().filter((r) => !r.waiting && m.ticked.has(rowKey(r)))),
    onCancel: () => {
      m.choosing = false;
      paint();
    },
    onForgetUnticked: () => void forget(pending().filter((r) => !r.waiting && !m.ticked.has(rowKey(r)))),
  };
  void listen<RememberedRow[]>("remembered", (e) => set(e.payload));
  void invoke<RememberedRow[]>("remembered_sessions").then((rows) => set(rows ?? [])).catch(() => undefined);
  // The Choose list's "N ago" moves with time.
  setInterval(() => {
    if (m.choosing) paint();
  }, 30_000);
}
