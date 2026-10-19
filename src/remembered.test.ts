import styles from "./styles.css?raw";
import { describe, expect, it, vi } from "vitest";

const tauri = vi.hoisted(() => ({
  calls: [] as { cmd: string; args: unknown }[],
  replies: {} as Record<string, unknown>,
  listeners: {} as Record<string, (e: { payload: unknown }) => void>,
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string, args?: unknown) => {
    tauri.calls.push({ cmd, args });
    return tauri.replies[cmd];
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, f: (e: { payload: unknown }) => void) => {
    tauri.listeners[name] = f;
    return () => undefined;
  }),
}));

import { bannerText, failuresText, initRemembered, renderBanner, resumeToast, rowKey, tildePath, type BannerModel } from "./remembered";
import type { RememberedRow } from "./types";

const NOW = Date.parse("2026-10-19T09:00:00Z");
const row = (id: string, machine: string | null, extra: Partial<RememberedRow> = {}): RememberedRow => ({ agent: "claude-code", sessionId: id, dir: `/Users/x/dev/${id}`, name: `${id}-name`, firstSeenMs: NOW - 3_600_000, lastSeenMs: NOW - 300_000, offered: true, machine, ...extra });
const handlers = () => ({ onResumeAll: vi.fn(), onChoose: vi.fn(), onDismiss: vi.fn(), onDismissFailed: vi.fn(), onTick: vi.fn(), onResumeChosen: vi.fn(), onCancel: vi.fn(), onForgetUnticked: vi.fn() });
const model = (rows: RememberedRow[], extra: Partial<BannerModel> = {}): BannerModel => ({ rows, choosing: false, ticked: new Set(), busy: false, ...extra });
const flush = () => new Promise((r) => setTimeout(r, 0));
const click = (root: ParentNode, action: string) => root.querySelector<HTMLButtonElement>(`button[data-action=${action}]`)!.click();

describe("banner text", () => {
  it("counts the sessions and splits them by machine only when more than one is involved", () => {
    const five = [row("a", null), row("b", null), row("c", null), row("g1", "Gnowee"), row("g2", "Gnowee")];
    expect(bannerText(five)).toBe("5 sessions were running when Maya last stopped · 3 here · 2 on Gnowee");
    expect(bannerText([row("a", null)])).toBe("1 session was running when Maya last stopped");
    expect(bannerText([row("a", "desk"), row("b", "desk")])).toBe("2 sessions were running when Maya last stopped");
  });

  it("shortens home folders to ~ on every platform", () => {
    expect(tildePath("/Users/x/dev/calibre")).toBe("~/dev/calibre");
    expect(tildePath("/home/tiago/dev")).toBe("~/dev");
    expect(tildePath("C:\\Users\\tiago\\dev")).toBe("~\\dev");
    expect(tildePath("/opt/work")).toBe("/opt/work");
  });

  it("names each failure with its reason", () => {
    expect(failuresText([row("raffinad-3c", null, { name: "raffinad-3c", error: "folder missing" })])).toBe("Couldn't resume 1: raffinad-3c (folder missing)");
  });

  it("sums up a resume, naming a single failure and the machines skipped", () => {
    expect(resumeToast([{ agent: "claude-code", sessionId: "a", name: "a" }, { agent: "claude-code", sessionId: "b", name: "b" }])).toBe("Resuming 2.");
    expect(resumeToast([{ agent: "claude-code", sessionId: "a", name: "a" }, { agent: "claude-code", sessionId: "g", name: "g", machine: "Gnowee", error: "waiting for Gnowee" }])).toBe("Resuming 1; skipped 1 waiting for Gnowee.");
    expect(resumeToast([{ agent: "codex", sessionId: "c", name: "calibre-88", error: "Codex is not installed" }])).toBe("Couldn't resume calibre-88: Codex is not installed");
    expect(resumeToast([])).toBe("Nothing to resume.");
  });
});

describe("renderBanner", () => {
  it("shows nothing for an empty list, and the three buttons otherwise", () => {
    expect(renderBanner(model([]), handlers(), NOW)).toBeNull();
    const h = handlers();
    const el = renderBanner(model([row("a", null)]), h, NOW)!;
    click(el, "resume-all");
    click(el, "choose");
    click(el, "dismiss");
    expect([h.onResumeAll, h.onChoose, h.onDismiss].every((f) => f.mock.calls.length === 1)).toBe(true);
  });

  it("lists the rows by machine, ticked, with a waiting machine's rows unticked and saying so", () => {
    const rows = [row("a", null), row("g1", "Gnowee", { waiting: true })];
    const h = handlers();
    const el = renderBanner(model(rows, { choosing: true, ticked: new Set([rowKey(rows[0])]) }), h, NOW)!;
    expect([...el.querySelectorAll(".remembered__machine")].map((m) => m.textContent)).toEqual(["Here", "Gnowee"]);
    const boxes = [...el.querySelectorAll<HTMLInputElement>(".remembered__row input")];
    expect(boxes.map((b) => [b.checked, b.disabled])).toEqual([[true, false], [false, true]]);
    expect(el.querySelector(".remembered__dir")!.textContent).toBe("~/dev/a");
    expect([...el.querySelectorAll(".remembered__age")].map((a) => a.textContent)).toEqual(["5m ago", "waiting for Gnowee"]);
    expect(el.querySelector<HTMLButtonElement>("button[data-action=resume-chosen]")!.textContent).toBe("Resume 1");
    click(el, "cancel");
    click(el, "forget-unticked");
    expect(h.onCancel).toHaveBeenCalled();
    expect(h.onForgetUnticked).toHaveBeenCalled();
    expect(el.querySelector("button[data-action=resume-all]")).toBeNull();
  });

  it("puts failures on their own line with a Dismiss", () => {
    const h = handlers();
    const el = renderBanner(model([row("lost", null, { name: "raffinad-3c", error: "folder missing" })]), h, NOW)!;
    expect(el.textContent).toContain("Couldn't resume 1: raffinad-3c (folder missing)");
    expect(el.querySelector("button[data-action=resume-all]")).toBeNull();
    click(el, "dismiss-failed");
    expect(h.onDismissFailed).toHaveBeenCalled();
  });
});

describe("initRemembered", () => {
  const start = async (rows: RememberedRow[]) => {
    tauri.calls = [];
    tauri.replies = { remembered_sessions: rows, resume_remembered: [], forget_remembered: null };
    const host = document.createElement("div");
    initRemembered(host);
    await flush();
    return host;
  };
  const sent = (cmd: string) => (tauri.calls.filter((c) => c.cmd === cmd).pop()?.args as { rows: RememberedRow[] }).rows.map(rowKey);

  it("Resume all sends every row not failed, waiting ones included so the toast can say so", async () => {
    const rows = [row("a", null), row("g", "Gnowee", { waiting: true }), row("x", null, { error: "folder missing" })];
    const host = await start(rows);
    click(host, "resume-all");
    await flush();
    expect(sent("resume_remembered")).toEqual([rowKey(rows[0]), rowKey(rows[1])]);
  });

  it("Choose sends the ticked rows to Resume and the unticked ones to Forget", async () => {
    const rows = [row("a", null), row("b", null), row("g", "Gnowee", { waiting: true })];
    const host = await start(rows);
    click(host, "choose");
    const b = host.querySelector<HTMLInputElement>(`[data-key="${rowKey(rows[1])}"] input`)!;
    b.checked = false;
    b.dispatchEvent(new Event("change"));
    expect(host.querySelector<HTMLButtonElement>("button[data-action=resume-chosen]")!.textContent).toBe("Resume 1");
    click(host, "forget-unticked");
    await flush();
    expect(sent("forget_remembered")).toEqual([rowKey(rows[1])]);
  });

  it("the same session id on two machines is ticked and resumed separately", async () => {
    const rows = [row("same", null), row("same", "Gnowee")];
    const host = await start(rows);
    click(host, "choose");
    const remote = host.querySelector<HTMLInputElement>(`[data-key="${rowKey(rows[1])}"] input`)!;
    remote.checked = false;
    remote.dispatchEvent(new Event("change"));
    expect(host.querySelector<HTMLInputElement>(`[data-key="${rowKey(rows[0])}"] input`)!.checked).toBe(true);
    click(host, "resume-chosen");
    await flush();
    expect(sent("resume_remembered")).toEqual([rowKey(rows[0])]);
  });

  it("repaints from the remembered event", async () => {
    const host = await start([]);
    expect(host.childElementCount).toBe(0);
    tauri.listeners.remembered({ payload: [row("a", null)] });
    expect(host.textContent).toContain("1 session was running when Maya last stopped");
  });
});

describe("the Choose list on screen", () => {
  it("renders every row and the three buttons, with the badge sized by the stylesheet", () => {
    const rows = [row("a", null), row("b", null, { agent: "codex" })];
    const el = renderBanner(model(rows, { choosing: true, ticked: new Set(rows.map(rowKey)) }), handlers(), NOW)!;
    expect(el.querySelectorAll(".remembered__row")).toHaveLength(2);
    for (const a of ["resume-chosen", "cancel", "forget-unticked"]) expect(el.querySelector(`button[data-action=${a}]`)).not.toBeNull();
    const badges = [...el.querySelectorAll(".remembered__row .remembered__agent")];
    expect(badges).toHaveLength(2);
    expect(styles).toMatch(/\.remembered__agent\b[^{]*\{[^}]*width: 14px;[^}]*height: 14px/);
    expect(styles).toMatch(/\.remembered__agent--text\b[^{]*\{[^}]*width: auto/);
  });
});
