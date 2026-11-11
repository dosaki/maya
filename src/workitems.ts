import { formatAge } from "./format";
import { iconElement } from "./icons";
import { morph } from "./morph";
import { STATE_LABEL, type Card, type ItemRepo, type StartCheck, type Started, type WorkItem, type WorkItemsState, type WorkStarted } from "./types";

export interface WorkFilters {
  source: string | null;
  /** `<forge>:<path>`, or null for every repository. */
  repo: string | null;
  status: string | null;
  search: string;
}

export type WorkItemAction =
  | { kind: "open-item" | "start"; key: string }
  | { kind: "session"; key: string; pid: number }
  | { kind: "settings"; group: "git" | "workItems" };

const repoKey = (i: WorkItem) => (i.repo ? `${i.repo.forge}:${i.repo.path}` : null);

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  n.className = className;
  if (text !== undefined) n.textContent = text;
  return n;
}

/**
 * The live session working on `item`: one named `issue <id>` (what Start
 * opens), or one inside the folder the item was started in. The most
 * recently changed match wins.
 */
export function sessionFor(item: WorkItem, started: Started[], cards: Card[]): Card | null {
  // Windows folders use `\`: compare both sides with `/`.
  const slashes = (p: string) => p.replace(/\\/g, "/").replace(/\/+$/, "");
  const found = started.find((s) => s.key === item.key)?.dir;
  const dir = found === undefined ? null : slashes(found);
  const matches = cards.filter((c) => {
    if (c.name === `issue ${item.id}`) return true;
    if (!dir) return false;
    const cwd = slashes(c.cwd);
    return cwd === dir || cwd.startsWith(`${dir}/`);
  });
  if (matches.length === 0) return null;
  return matches.sort((a, b) => b.stateSince - a.stateSince)[0];
}

/** Items with a session go to "Being worked on" (in list order), the rest are still to pick. */
export function areas(state: WorkItemsState, cards: Card[]): { toPick: WorkItem[]; working: { item: WorkItem; card: Card }[] } {
  const toPick: WorkItem[] = [];
  const working: { item: WorkItem; card: Card }[] = [];
  for (const item of state.items) {
    const card = sessionFor(item, state.started, cards);
    if (card) working.push({ item, card });
    else toPick.push(item);
  }
  return { toPick, working };
}

export function applyWorkFilters(items: WorkItem[], f: WorkFilters): WorkItem[] {
  const words = f.search.trim().toLowerCase();
  return items.filter(
    (i) =>
      (f.source === null || i.source === f.source) &&
      (f.repo === null || repoKey(i) === f.repo) &&
      (f.status === null || i.status === f.status) &&
      (words === "" || [i.id, i.title, ...i.labels].some((t) => t.toLowerCase().includes(words))),
  );
}

function button(label: string, action: string, className = "card__btn"): HTMLButtonElement {
  const b = el("button", className, label);
  b.type = "button";
  b.dataset.action = action;
  return b;
}

export function renderItemCard(item: WorkItem, nowMs: number, card: Card | null, starting = false): HTMLElement {
  const root = el("article", card ? `item item--${card.state}` : "item");
  root.dataset.key = item.key;
  root.dataset.row = item.key;
  const head = el("header", "pr__head");
  const name = el("span", "pr__name");
  if (item.source === "github" || item.source === "gitlab") {
    const mark = iconElement(item.source, 12);
    mark.classList.add("pr__forge");
    mark.removeAttribute("aria-hidden");
    mark.setAttribute("role", "img");
    mark.setAttribute("aria-label", item.source === "gitlab" ? "GitLab" : "GitHub");
    name.append(mark);
  } else {
    name.append(el("span", "item__src", item.source));
  }
  const id = button(item.id, "open-item", "item__id");
  id.title = item.url;
  name.append(id);
  const chips = el("span", "pr__chips");
  for (const l of item.labels) chips.append(el("span", "pr__chip", l));
  if (item.status) chips.append(el("span", "item__status", item.status));
  head.append(name, chips);
  root.append(head, el("div", "pr__title", item.title));
  const foot = el("div", "card__actions");
  if (card) {
    if (card.snippet) root.append(el("div", "item__last", card.snippet));
    const live = el("span", "item__live");
    live.append(el("span", `item__dot item__dot--${card.state}`), STATE_LABEL[card.state]);
    const open = button("Open session", "session");
    open.dataset.pid = String(card.pid);
    foot.append(live, open);
  } else {
    const updated = Date.parse(item.updatedAt);
    const age = Number.isFinite(updated) ? `${formatAge(updated, nowMs)} ago` : "?";
    const meta =
      item.repo === null || item.tracker
        ? el("span", "item__meta item__meta--pick", "No repo: Maya picks")
        : el("span", "item__meta", `${item.assignees.length > 0 ? item.assignees.join(", ") : "Unassigned"} · ${age}`);
    const start = button("Start", "start", "card__btn card__btn--primary");
    start.disabled = starting;
    foot.append(meta, start);
  }
  root.append(foot);
  return root;
}

function area(kind: "pick" | "working", title: string, cards: HTMLElement[], empty: string): HTMLElement {
  const root = el("section", `issues__area issues__area--${kind}`);
  const head = el("div", "issues__head");
  head.append(el("h3", "", title), el("span", "issues__count", String(cards.length)));
  const col = el("div", "issues__col");
  if (cards.length === 0) col.append(el("div", "reviews__empty", empty));
  else col.append(...cards);
  root.append(head, col);
  return root;
}

function fillSelect(sel: HTMLSelectElement, options: { value: string; label: string }[]): void {
  sel.replaceChildren(
    ...options.map(({ value, label }) => {
      const o = document.createElement("option");
      o.value = value;
      o.textContent = label;
      return o;
    }),
  );
}

const SOURCE_LABEL: Record<string, string> = { github: "GitHub", gitlab: "GitLab" };

/**
 * The Issues pane in `host`: the filters and search, built once so an open
 * dropdown survives the repaints, over the two areas that `update` repaints
 * in place.
 */
export function makeWorkItemsView(host: HTMLElement) {
  let starting: ReadonlySet<string> = new Set();
  const filters: WorkFilters = { source: null, repo: null, status: null, search: "" };
  let state: WorkItemsState = { items: [], started: [], errors: [], sourceLabels: {}, gitOff: false, off: false, fetchedAt: null };
  let cards: Card[] = [];
  let nowMs = Date.now();
  let optionsKey: string | null = null;

  const select = (name: string, label: string, set: (v: string | null) => void) => {
    const s = el("select", "reviews__filter");
    s.name = name;
    s.setAttribute("aria-label", label);
    s.addEventListener("change", () => {
      set(s.value || null);
      paint();
    });
    return s;
  };
  const sourceSel = select("source", "Source", (v) => (filters.source = v));
  const repoSel = select("repo", "Repository", (v) => (filters.repo = v));
  const statusSel = select("status", "Status", (v) => (filters.status = v));
  const search = el("input", "issues__search");
  search.type = "text";
  search.name = "search";
  search.placeholder = "Search issues";
  search.setAttribute("aria-label", "Search issues");
  search.addEventListener("input", () => {
    filters.search = search.value;
    paint();
  });
  const fetched = el("span", "issues__fetched");
  const bar = el("div", "issues__bar");
  bar.append(sourceSel, repoSel, statusSel, search, el("span", "tabs__spacer"), fetched);
  const body = el("div", "issues__body");
  host.replaceChildren(bar, body);

  function paint(): void {
    if (state.off) {
      bar.hidden = true;
      // Git off entirely, or on with both services off in Work Items.
      const group = state.gitOff ? "git" : "workItems";
      const off = el("div", "reviews__off", `Issues are off. Turn on GitHub or GitLab in Settings → ${state.gitOff ? "Git" : "Work Items"}. `);
      const open = button("Open Settings", "settings");
      open.dataset.group = group;
      off.append(open);
      const fresh = el("div", "issues__body");
      fresh.append(off);
      morph(body, fresh);
      return;
    }
    bar.hidden = false;
    const sources = [...new Set(state.items.map((i) => i.source))].sort();
    const repos = [...new Map(state.items.flatMap((i) => (i.repo ? [[repoKey(i)!, i.repo.path] as const] : []))).entries()].sort((a, b) => a[1].localeCompare(b[1]));
    const statuses = [...new Set(state.items.flatMap((i) => (i.status ? [i.status] : [])))].sort();
    if (filters.source !== null && !sources.includes(filters.source)) filters.source = null;
    if (filters.repo !== null && !repos.some(([k]) => k === filters.repo)) filters.repo = null;
    if (filters.status !== null && !statuses.includes(filters.status)) filters.status = null;
    const key = JSON.stringify([sources, repos, statuses]);
    if (key !== optionsKey) {
      fillSelect(sourceSel, [{ value: "", label: "All Sources" }, ...sources.map((s) => ({ value: s, label: SOURCE_LABEL[s] ?? s }))]);
      fillSelect(repoSel, [{ value: "", label: "All Repositories" }, ...repos.map(([value, label]) => ({ value, label }))]);
      fillSelect(statusSel, [{ value: "", label: "Any Status" }, ...statuses.map((s) => ({ value: s, label: s }))]);
      optionsKey = key;
    }
    sourceSel.value = filters.source ?? "";
    repoSel.value = filters.repo ?? "";
    statusSel.value = filters.status ?? "";
    fetched.textContent = state.fetchedAt === null ? "Checking…" : `Checked ${formatAge(state.fetchedAt, nowMs)} ago`;

    const { toPick, working } = areas(state, cards);
    const shownPick = applyWorkFilters(toPick, filters);
    const keep = new Set(applyWorkFilters(working.map((w) => w.item), filters).map((i) => i.key));
    const shownWorking = working.filter((w) => keep.has(w.item.key));
    const fresh = el("div", "issues__body");
    for (const e of state.errors) fresh.append(el("div", "reviews__error issues__error", e.message));
    const cols = el("div", "issues__cols");
    const emptyPick = state.fetchedAt === null ? "Checking…" : state.items.length === 0 ? "No issues match your rules." : "No issues match these filters.";
    cols.append(
      area("pick", "To pick", shownPick.map((i) => renderItemCard(i, nowMs, null, starting.has(i.key))), emptyPick),
      area("working", "Being worked on", shownWorking.map((w) => renderItemCard(w.item, nowMs, w.card)), "Nothing started yet."),
    );
    fresh.append(cols);
    morph(body, fresh);
  }

  return {
    update(next: WorkItemsState, nextCards: Card[], now: number, startingKeys: ReadonlySet<string> = new Set()): void {
      state = next;
      starting = startingKeys;
      cards = nextCards;
      nowMs = now;
      paint();
    },
  };
}

/** What a click in the Issues pane means: one of the buttons, or nothing. */
export function workItemActionFor(target: Element): WorkItemAction | null {
  const btn = target.closest<HTMLElement>("button[data-action]");
  if (!btn) return null;
  if (btn.dataset.action === "settings") return { kind: "settings", group: btn.dataset.group === "git" ? "git" : "workItems" };
  const key = btn.closest<HTMLElement>(".item")?.dataset.key;
  if (!key) return null;
  const kind = btn.dataset.action;
  if (kind === "session") return { kind, key, pid: Number(btn.dataset.pid) };
  if (kind === "open-item" || kind === "start") return { kind, key };
  return null;
}

const repoValue = (r: ItemRepo) => `${r.forge}:${r.path}`;
const repoLabel = (r: ItemRepo) => `${r.path} (${r.forge === "gitlab" ? "GitLab" : "GitHub"})`;

/** True when Start has something to ask: a takeover to confirm or a repository to pick. */
export function needsDialog(check: StartCheck): boolean {
  return check.takeover.length > 0 || check.pick !== null;
}

/** The question before a Start: confirm a takeover and/or confirm or change Maya's repository pick. */
export function renderStartDialog(item: WorkItem, check: StartCheck, h: { onStart(repo: ItemRepo | null): void; onCancel(): void }): HTMLElement {
  const root = el("div", "modal");
  root.append(el("div", "modal__backdrop"));
  const panel = el("section", "modal__panel modal__panel--compact");
  panel.setAttribute("role", "dialog");
  panel.setAttribute("aria-modal", "true");
  const head = el("header", "modal__head");
  const titles = el("div", "modal__titles");
  titles.append(el("h2", "modal__title", `Start ${item.id}: ${item.title}`));
  head.append(titles);
  const body = el("div", "modal__setup");
  const cancel = el("button", "card__btn", "Cancel");
  cancel.type = "button";
  cancel.addEventListener("click", () => h.onCancel());
  const buttons = el("div", "startdialog__buttons");
  if (check.pick && check.pick.options.length === 0) {
    // Nothing to choose from, so nothing to start.
    body.append(el("p", "", "Maya knows no repositories yet. Name one in a rule in Settings → Work Items, or clone it into the clones folder first."));
    buttons.append(cancel);
    body.append(buttons);
    panel.append(head, body);
    root.append(panel);
    return root;
  }
  if (check.takeover.length > 0) {
    body.append(el("p", "", `This issue is already assigned to ${check.takeover.join(", ")}. Are you sure you want to take over?`));
  }
  const start = el("button", "card__btn card__btn--primary", check.takeover.length > 0 ? "Yes, take over and start" : "Start session");
  start.type = "button";
  let repoSel: HTMLSelectElement | null = null;
  const options = check.pick?.options ?? [];
  if (check.pick) {
    const { picked, why } = check.pick;
    body.append(el("p", "", picked ? `Maya picked ${picked.path} — ${why}` : "Maya could not tell which repository; choose one."));
    const sel = el("select", "startdialog__repo");
    sel.name = "repo";
    sel.setAttribute("aria-label", "Repository");
    if (!picked) {
      const none = document.createElement("option");
      none.value = "";
      none.textContent = "Choose a repository";
      sel.append(none);
    }
    for (const r of options) {
      const o = document.createElement("option");
      o.value = repoValue(r);
      o.textContent = repoLabel(r);
      sel.append(o);
    }
    sel.value = picked ? repoValue(picked) : "";
    sel.addEventListener("change", () => (start.disabled = sel.value === ""));
    start.disabled = sel.value === "";
    repoSel = sel;
    body.append(sel);
  }
  body.append(el("p", "startdialog__branch", `Branch: ${check.branch}`));
  start.addEventListener("click", () => {
    if (!repoSel) return h.onStart(null);
    h.onStart(options.find((r) => repoValue(r) === repoSel!.value) ?? null);
  });
  buttons.append(cancel, start);
  body.append(buttons);
  panel.append(head, body);
  root.append(panel);
  return root;
}

/**
 * Runs `run` for `key` unless a Start for it is already in flight; the key
 * stays in `inFlight` until `run` settles, and `changed` fires on each edit.
 */
export async function guardStart(inFlight: Set<string>, key: string, run: () => Promise<void>, changed: () => void = () => {}): Promise<void> {
  if (inFlight.has(key)) return;
  inFlight.add(key);
  changed();
  try {
    await run();
  } finally {
    inFlight.delete(key);
    changed();
  }
}

/** Checks, asks when needed, starts; toasts the outcome (or the error) and reports the opened folder. */
export async function startWorkItem(
  item: WorkItem,
  deps: {
    invoke: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
    host: HTMLElement;
    toast(s: string): void;
    opened(nativeTerm: number | null, dir: string): void;
  },
): Promise<void> {
  const { invoke, host, toast } = deps;
  const busy = "Close the open window first.";
  if (host.childElementCount > 0) return toast(busy);
  try {
    const check = await invoke<StartCheck>("work_item_check", { key: item.key });
    let repo: ItemRepo | null = null;
    if (needsDialog(check)) {
      // Something else may have opened a window while the check ran.
      if (host.childElementCount > 0) return toast(busy);
      const chosen = await new Promise<{ repo: ItemRepo | null } | null>((resolve) => {
        const close = (v: { repo: ItemRepo | null } | null) => {
          document.removeEventListener("keydown", onKey, true);
          gone.disconnect();
          if (dialog.parentNode === host) host.replaceChildren();
          resolve(v);
        };
        const onKey = (e: KeyboardEvent) => {
          if (e.key !== "Escape") return;
          e.stopPropagation();
          close(null);
        };
        const dialog = renderStartDialog(item, check, { onStart: (r) => close({ repo: r }), onCancel: () => close(null) });
        // Other code writes the host too: a dialog replaced under us counts as a cancel.
        const gone = new MutationObserver(() => {
          if (dialog.parentNode !== host) close(null);
        });
        document.addEventListener("keydown", onKey, true);
        host.replaceChildren(dialog);
        gone.observe(host, { childList: true });
      });
      if (!chosen) return;
      repo = chosen.repo;
    }
    const r = await invoke<WorkStarted>("work_item_start", { key: item.key, repo, takeOver: check.takeover });
    // A notice (the issue not updated, say) shows in the line above the areas.
    toast(`Working on ${item.id} in ${r.dir.split("/").filter(Boolean).pop() ?? r.dir}`);
    deps.opened(r.nativeTerm, r.dir);
  } catch (e) {
    toast(String(e));
  }
}
