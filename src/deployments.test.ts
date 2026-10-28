import { describe, expect, it } from "vitest";
import { approvalCount, deploymentActionFor, renderDeployments, withoutRun, type RowUi } from "./deployments";
import type { DeployBlocker, DeploymentRun, DeploymentState } from "./types";

const NOW = Date.parse("2026-09-29T10:00:00Z");

const run = (runId: number, wait: DeploymentRun["wait"]): DeploymentRun => ({
  repo: "O/r",
  runId,
  runNumber: 40 + runId,
  workflow: "Deploy",
  branch: "main",
  sha: "0123456789abcdef",
  title: `change ${runId}`,
  actor: "jane",
  url: `https://github.com/O/r/actions/runs/${runId}`,
  since: "2026-09-29T08:00:00Z",
  wait,
});
const approval = (runId: number, envs: string[]) => run(runId, { kind: "approval", environments: envs.map((name, id) => ({ id, name })) });
const blocked = (runId: number, by: DeployBlocker | null) => run(runId, { kind: "blocked", by });
const state = (runs: DeploymentRun[], over: Partial<DeploymentState> = {}): DeploymentState => ({
  runs,
  error: null,
  off: false,
  assistant: false,
  fetchedAt: 1,
  ...over,
});
const ui = (busy: number[] = [], errors: [number, string][] = []): RowUi => ({ busy: new Set(busy), errors: new Map(errors) });

describe("renderDeployments", () => {
  it("paints an approval row with its environments and both buttons", () => {
    const root = renderDeployments(state([approval(7, ["production"])]), NOW, ui());
    expect(root.querySelector(".pr__chip")?.textContent).toBe("production");
    expect([...root.querySelectorAll("button[data-action=reject], button[data-action=approve]")].map((b) => b.textContent)).toEqual(["Reject", "Approve"]);
    const row = root.querySelector<HTMLElement>("[data-run-id='7']")!;
    expect(row.dataset.repo).toBe("O/r");
    expect(row.querySelector(".pr__meta")?.textContent).toBe("jane · waiting 2h · O/r");
    expect(row.querySelector("[data-action=open-run]")?.textContent).toBe("Deploy");
    expect(row.querySelector(".pr__title")?.textContent).toBe("change 7");
  });

  it("paints a blocked row with what it waits on and no buttons", () => {
    const root = renderDeployments(state([approval(7, ["production"]), blocked(9, { runId: 7, sha: "a1b2c3d4e5", title: "fix login redirect", url: "u" })]), NOW, ui());
    const row = root.querySelector<HTMLElement>("[data-run-id='9']")!;
    expect(row.classList.contains("deploy--blocked")).toBe(true);
    expect(row.querySelector("[data-action=open-blocker]")?.textContent).toBe("Waiting on a1b2c3d fix login redirect");
    expect(row.querySelector("[data-action=approve]")).toBeNull();
    expect(row.querySelector("[data-action=reject]")).toBeNull();
    expect(row.querySelector(".pr__chip")).toBeNull();
  });

  it("says waiting on another deployment when the blocker is unknown", () => {
    const root = renderDeployments(state([blocked(9, null)]), NOW, ui());
    const row = root.querySelector<HTMLElement>("[data-run-id='9']")!;
    expect(row.textContent).toContain("Waiting on another deployment");
    expect(row.querySelector("[data-action=open-blocker]")).toBeNull();
  });

  it("disables a busy row's buttons and shows its error", () => {
    const root = renderDeployments(state([approval(7, ["production"]), approval(8, ["production"])]), NOW, ui([7], [[7, "403: nope"]]));
    const mine = root.querySelector<HTMLElement>("[data-run-id='7']")!;
    expect(mine.querySelector<HTMLButtonElement>("[data-action=approve]")!.disabled).toBe(true);
    expect(mine.querySelector<HTMLButtonElement>("[data-action=reject]")!.disabled).toBe(true);
    expect(mine.querySelector(".deploy__error")?.textContent).toBe("403: nope");
    const other = root.querySelector<HTMLElement>("[data-run-id='8']")!;
    expect(other.querySelector<HTMLButtonElement>("[data-action=approve]")!.disabled).toBe(false);
    expect(other.querySelector(".deploy__error")).toBeNull();
  });

  it("shows the empty, checking, off and assistant states", () => {
    expect(renderDeployments(state([]), NOW, ui()).textContent).toBe("No deployments are waiting on you.");
    expect(renderDeployments(state([], { fetchedAt: null }), NOW, ui()).textContent).toBe("Checking…");
    const off = renderDeployments(state([], { off: true }), NOW, ui());
    expect(off.textContent).toBe("GitHub is off. Turn it on in Settings → Git. Open Settings");
    expect(off.querySelector("button[data-action=git-settings]")?.textContent).toBe("Open Settings");
    expect(renderDeployments(state([], { assistant: true, off: true }), NOW, ui()).textContent).toBe("Deployments are checked on the main.");
    const err = renderDeployments(state([approval(7, ["p"])], { error: "rate limited" }), NOW, ui());
    expect(err.querySelector(".reviews__error")?.textContent).toBe("rate limited");
    expect(err.firstElementChild?.classList.contains("reviews__error")).toBe(true);
  });
});

describe("deploymentActionFor", () => {
  it("reads a click into an action", () => {
    const root = renderDeployments(state([approval(7, ["production"]), blocked(9, { runId: 7, sha: "a1b2c3d4e5", title: "t", url: "u" })]), NOW, ui());
    const pick = (sel: string) => root.querySelector<HTMLElement>(sel)!;
    expect(deploymentActionFor(pick("[data-run-id='7'] [data-action=approve]"))).toEqual({ kind: "approve", repo: "O/r", runId: 7 });
    expect(deploymentActionFor(pick("[data-run-id='7'] [data-action=reject]"))).toEqual({ kind: "reject", repo: "O/r", runId: 7 });
    expect(deploymentActionFor(pick("[data-run-id='7'] [data-action=open-run]"))).toEqual({ kind: "open-run", repo: "O/r", runId: 7 });
    expect(deploymentActionFor(pick("[data-run-id='9'] [data-action=open-blocker]"))).toEqual({ kind: "open-blocker", repo: "O/r", runId: 9 });
    expect(deploymentActionFor(pick("[data-run-id='7'] .pr__title"))).toBeNull();
    const off = renderDeployments(state([], { off: true }), NOW, ui());
    expect(deploymentActionFor(off.querySelector("button")!)).toEqual({ kind: "git-settings" });
  });
});

describe("approvalCount", () => {
  it("counts only approval rows", () => {
    expect(approvalCount([approval(7, ["p"]), blocked(9, null)])).toBe(1);
  });
});

describe("withoutRun", () => {
  it("drops only the run with that repo and id", () => {
    const other = { ...approval(7, ["p"]), repo: "O/other" };
    const s = state([approval(7, ["p"]), other, blocked(9, null)], { error: "e" });
    const next = withoutRun(s, "O/r", 7);
    expect(next.runs.map((r) => `${r.repo}#${r.runId}`)).toEqual(["O/other#7", "O/r#9"]);
    expect(next.error).toBe("e");
    expect(s.runs).toHaveLength(3);
  });
});
