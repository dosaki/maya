import { formatAge } from "./format";
import type { DeploymentRun, DeploymentState } from "./types";

/** What the pane remembers per run between repaints: a click in flight, and why the last one failed. */
export interface RowUi {
  busy: Set<number>;
  errors: Map<number, string>;
}

export type DeploymentAction =
  | { kind: "approve" | "reject" | "open-run" | "open-blocker"; repo: string; runId: number }
  | { kind: "git-settings" };

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  n.className = className;
  if (text !== undefined) n.textContent = text;
  return n;
}

function button(className: string, action: string, text: string): HTMLButtonElement {
  const b = el("button", className, text);
  b.type = "button";
  b.dataset.action = action;
  return b;
}

/** The runs that wait on the user's click; blocked runs wait on another run. */
export function approvalCount(runs: DeploymentRun[]): number {
  return runs.filter((r) => r.wait.kind === "approval").length;
}

function renderRun(run: DeploymentRun, nowMs: number, ui: RowUi): HTMLElement {
  const wait = run.wait;
  const root = el("article", wait.kind === "blocked" ? "pr deploy deploy--blocked" : "pr deploy");
  root.dataset.repo = run.repo;
  root.dataset.runId = String(run.runId);
  const head = el("header", "pr__head");
  const name = el("span", "pr__name");
  name.append(el("span", "pr__repo", run.repo.split("/").pop() ?? run.repo));
  const open = button("deploy__link", "open-run", run.workflow);
  open.title = run.url;
  name.append(open, el("span", "deploy__branch", run.branch));
  const chips = el("span", "pr__chips");
  if (wait.kind === "approval") for (const env of wait.environments) chips.append(el("span", "pr__chip", env.name));
  head.append(name, chips);
  root.append(head, el("div", "pr__title", run.title));
  if (wait.kind === "blocked") {
    if (wait.by) {
      const by = button("deploy__link", "open-blocker", `Waiting on ${wait.by.sha.slice(0, 7)} ${wait.by.title}`);
      by.title = wait.by.url;
      root.append(by);
    } else {
      root.append(el("span", "pr__meta", "Waiting on another deployment"));
    }
  }
  const since = Date.parse(run.since);
  root.append(el("div", "pr__meta", `${run.actor} · waiting ${Number.isFinite(since) ? formatAge(since, nowMs) : "?"} · ${run.repo}`));
  if (wait.kind === "approval") {
    const busy = ui.busy.has(run.runId);
    const error = ui.errors.get(run.runId);
    if (error) root.append(el("div", "deploy__error", error));
    const actions = el("div", "card__actions");
    const reject = button("card__btn", "reject", "Reject");
    const approve = button("card__btn card__btn--primary", "approve", "Approve");
    reject.disabled = approve.disabled = busy;
    actions.append(reject, approve);
    root.append(actions);
  }
  return root;
}

/**
 * The Deployments pane: why it is empty (the main Maya checks, GitHub is off,
 * still checking, nothing waiting) or the last fetch's error above the runs.
 */
export function renderDeployments(state: DeploymentState, nowMs: number, ui: RowUi): HTMLElement {
  const root = el("section", "reviews");
  if (state.assistant) {
    root.append(el("div", "reviews__empty", "Deployments are checked on the main."));
    return root;
  }
  if (state.off) {
    const off = el("div", "reviews__off", "GitHub is off. Turn it on in Settings → Git. ");
    off.append(button("card__btn", "git-settings", "Open Settings"));
    root.append(off);
    return root;
  }
  if (state.error) root.append(el("div", "reviews__error", state.error));
  if (state.runs.length === 0) {
    root.append(el("div", "reviews__empty", state.fetchedAt === null ? "Checking…" : "No deployments are waiting on you."));
    return root;
  }
  const list = el("div", "reviews__list");
  for (const run of state.runs) list.append(renderRun(run, nowMs, ui));
  root.append(list);
  return root;
}

/** What a click in the Deployments pane means: one of its buttons, or nothing. */
export function deploymentActionFor(target: Element): DeploymentAction | null {
  const btn = target.closest<HTMLElement>("button[data-action]");
  if (btn?.dataset.action === "git-settings") return { kind: "git-settings" };
  const row = target.closest<HTMLElement>(".deploy");
  const kind = btn?.dataset.action;
  if (!row?.dataset.repo || !row.dataset.runId) return null;
  if (kind !== "approve" && kind !== "reject" && kind !== "open-run" && kind !== "open-blocker") return null;
  return { kind, repo: row.dataset.repo, runId: Number(row.dataset.runId) };
}
