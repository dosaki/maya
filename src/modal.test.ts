import styles from "./styles.css?raw";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
const live = vi.hoisted(() => ({ mounts: [] as { host: HTMLElement; pid?: number; term?: number; dispose: ReturnType<typeof vi.fn>; focus: ReturnType<typeof vi.fn> }[] }));
vi.mock("./live", () => ({
  mountLive: (host: HTMLElement, target: { pid?: number; term?: number }) => {
    const m = { host, pid: target.pid, term: target.term, dispose: vi.fn(), focus: vi.fn() };
    live.mounts.push(m);
    return m;
  },
}));

import { MAX_ATTACHMENT_BYTES, PANEL_SIZE_KEY, openTermViewer, renderTermViewer, RESIZE_CLICK_GRACE_MS, anchorPanel, applyStage, attachmentTooLarge, closeModal, cursorFor, draggedPanelSize, edgeDrag, edgesAt, fitAnchoredPanel, fitPosition, historyRefetchDue, isLive, isTerminalCommand, openModal, parsePanelSize, patchModal, refreshModal, renderModal, replyStatus, setComposerOptions, setViewerTheme, statusLine, workingFor } from "./modal";
import { comboLabel, defaultKeys, setKeyBindings } from "./keys";
import type { Card, Turn } from "./types";

const base: Card = { sessionId: "s", pid: 1, name: "eye-1", cwd: "/x/dev/eye", state: "idle", stateSince: 0, snippet: "", awaiting: null, hasInbox: true, harness: "claude-code", pr: null, context: null };
const asking: Card = { ...base, state: "awaiting", stateSince: 5000,
  awaiting: { kind: "question", detail: "Q?", questions: [{ question: "Q?", header: "H", multiSelect: false, options: [{ label: "A", description: "desc a" }], own: { label: "Type something", row: 1 }, chatRow: 2 }] } };
const turns: Turn[] = [{ kind: "user", text: "hi" }, { kind: "assistant", text: "hello" }, { kind: "tool", text: "Bash: ls" }, { kind: "peer", text: "from eye" }];
const handlers = () => ({ onSend: vi.fn(), onTerminal: vi.fn(), onClose: vi.fn(), onPick: vi.fn(), onStop: vi.fn(), onSetOption: vi.fn(), onCycleMode: vi.fn(), onOpenPr: vi.fn(), onRename: vi.fn(), onCommand: vi.fn(), onCompact: vi.fn(), onOpenLink: vi.fn(), onPasteFiles: vi.fn() });

describe("historyRefetchDue", () => {
  it("always refetches a local card, and a remote one only when its state, state time or snippet moved", () => {
    const due = (before: Card, fresh: Card) => historyRefetchDue(before, fresh, 0, 1_000);
    expect(due(base, { ...base })).toBe(true);
    const remote = { ...base, machine: "laptop" };
    expect(due(remote, { ...remote })).toBe(false);
    expect(due(remote, { ...remote, stale: true, context: { used: 1, window: 2, percent: 50 } })).toBe(false);
    expect(due(remote, { ...remote, state: "working" })).toBe(true);
    expect(due(remote, { ...remote, stateSince: 5 })).toBe(true);
    expect(due(remote, { ...remote, snippet: "done" })).toBe(true);
  });

  it("refetches a working remote card every 3 s even when nothing on it moved", () => {
    const working = { ...base, machine: "laptop", state: "working" as const };
    expect(historyRefetchDue(working, { ...working }, 10_000, 12_999)).toBe(false);
    expect(historyRefetchDue(working, { ...working }, 10_000, 13_000)).toBe(true);
    const idle = { ...base, machine: "laptop" };
    expect(historyRefetchDue(idle, { ...idle }, 10_000, 60_000)).toBe(false);
    expect(historyRefetchDue(base, { ...base }, 10_000, 10_001)).toBe(true);
  });
});

const rect = (left: number, top: number, width: number, height: number) => ({ left, top, right: left + width, bottom: top + height, width, height, x: left, y: top, toJSON: () => "" }) as DOMRect;

describe("panel size", () => {
  it("keeps a stored size only when it is two numbers at least the minimum", () => {
    expect(parsePanelSize(null)).toBeNull();
    expect(parsePanelSize("nonsense")).toBeNull();
    expect(parsePanelSize(JSON.stringify({ width: 900.4, height: 700.6 }))).toEqual({ width: 900, height: 701 });
    expect(parsePanelSize(JSON.stringify({ width: 100, height: 700 }))).toBeNull();
    expect(parsePanelSize(JSON.stringify({ width: "wide", height: 700 }))).toBeNull();
    expect(parsePanelSize(JSON.stringify({ width: Infinity, height: 700 }))).toBeNull();
  });

  it("reads the dragged size from the panel's inline style, never the rendered one", () => {
    const panel = document.createElement("section");
    expect(draggedPanelSize(panel)).toBeNull();
    panel.style.width = "812px";
    panel.style.height = "640.5px";
    expect(draggedPanelSize(panel)).toEqual({ width: 812, height: 641 });
  });

  it("finds the edges under the pointer in a band just outside and just inside the panel", () => {
    const r = rect(100, 50, 800, 600);
    expect(edgesAt(r, 500, 300)).toBeNull();
    expect(edgesAt(r, 500, 30)).toBeNull();
    expect(edgesAt(r, 96, 300)).toEqual({ left: true, right: false, top: false, bottom: false });
    expect(edgesAt(r, 103, 300)).toEqual({ left: true, right: false, top: false, bottom: false });
    expect(edgesAt(r, 500, 652)).toEqual({ left: false, right: false, top: false, bottom: true });
    expect(edgesAt(r, 903, 653)).toEqual({ left: false, right: true, top: false, bottom: true });
    expect(edgesAt(r, 98, 48)).toEqual({ left: true, right: false, top: true, bottom: false });
    expect(cursorFor({ left: true, right: false, top: false, bottom: false })).toBe("ew-resize");
    expect(cursorFor({ left: false, right: false, top: true, bottom: false })).toBe("ns-resize");
    expect(cursorFor({ left: true, right: false, top: true, bottom: false })).toBe("nwse-resize");
    expect(cursorFor({ left: false, right: true, top: false, bottom: true })).toBe("nwse-resize");
    expect(cursorFor({ left: false, right: true, top: true, bottom: false })).toBe("nesw-resize");
  });

  it("moves only the dragged edges, never below the minimum or past the window", () => {
    const start = { left: 240, top: 110, width: 800, height: 600 };
    const vp = { width: 1280, height: 820 };
    const none = { left: false, right: false, top: false, bottom: false };
    expect(edgeDrag(start, { ...none, left: true }, -40, 99, vp)).toEqual({ left: 200, top: 110, width: 840, height: 600 });
    expect(edgeDrag(start, { ...none, right: true }, 30, 99, vp)).toEqual({ left: 240, top: 110, width: 830, height: 600 });
    expect(edgeDrag(start, { ...none, top: true }, 99, -30, vp)).toEqual({ left: 240, top: 80, width: 800, height: 630 });
    expect(edgeDrag(start, { ...none, bottom: true }, 99, 50, vp)).toEqual({ left: 240, top: 110, width: 800, height: 650 });
    expect(edgeDrag(start, { ...none, left: true, top: true }, -500, -500, vp)).toEqual({ left: 0, top: 0, width: 1040, height: 710 });
    expect(edgeDrag(start, { ...none, left: true, top: true }, 900, 900, vp)).toEqual({ left: 560, top: 350, width: 480, height: 360 });
    expect(edgeDrag(start, { ...none, right: true, bottom: true }, 900, 900, vp)).toEqual({ left: 240, top: 110, width: 1040, height: 710 });
    expect(edgeDrag(start, { ...none, right: true, bottom: true }, -900, -900, vp)).toEqual({ left: 240, top: 110, width: 480, height: 360 });
  });

  it("shows the resize cursor on an edge, drags it with the panel pinned, and does not close on the drag's release", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    const panel = el.querySelector<HTMLElement>(".modal__panel")!;
    el.getBoundingClientRect = () => rect(0, 0, 1280, 820);
    panel.getBoundingClientRect = () => rect(240, 110, 800, 600);
    el.dispatchEvent(new MouseEvent("pointermove", { clientX: 500, clientY: 300, bubbles: true }));
    expect([el.style.cursor, el.classList.contains("modal--edge")]).toEqual(["", false]);
    el.dispatchEvent(new MouseEvent("pointermove", { clientX: 1043, clientY: 300, bubbles: true }));
    expect([el.style.cursor, el.classList.contains("modal--edge")]).toEqual(["ew-resize", true]);
    const now = vi.spyOn(Date, "now").mockReturnValue(10_000);
    Object.assign(window, { innerWidth: 1280, innerHeight: 820 });
    el.dispatchEvent(new MouseEvent("pointerdown", { clientX: 1043, clientY: 300, button: 0, bubbles: true }));
    expect(el.classList.contains("modal--anchored")).toBe(true);
    el.dispatchEvent(new MouseEvent("pointermove", { clientX: 1143, clientY: 350, bubbles: true }));
    expect([panel.style.left, panel.style.top, panel.style.width, panel.style.height]).toEqual(["240px", "110px", "900px", "600px"]);
    el.dispatchEvent(new MouseEvent("pointerup", { clientX: 1143, clientY: 350, bubbles: true }));
    el.querySelector<HTMLElement>(".modal__backdrop")!.click();
    expect(h.onClose).not.toHaveBeenCalled();
    now.mockReturnValue(10_000 + RESIZE_CLICK_GRACE_MS);
    el.querySelector<HTMLElement>(".modal__backdrop")!.click();
    expect(h.onClose).toHaveBeenCalledTimes(1);
    // The release ended the drag: a later move changes nothing.
    el.dispatchEvent(new MouseEvent("pointermove", { clientX: 600, clientY: 400, bubbles: true }));
    expect(panel.style.width).toBe("900px");
    now.mockRestore();
  });

  it("moves a pinned panel back inside a window that shrank, without touching its size", () => {
    expect(fitPosition({ left: 560, top: 300, width: 480, height: 360 }, { width: 800, height: 600 })).toEqual({ left: 312, top: 232 });
    expect(fitPosition({ left: 100, top: 50, width: 480, height: 360 }, { width: 1280, height: 820 })).toEqual({ left: 100, top: 50 });
    expect(fitPosition({ left: 100, top: 50, width: 900, height: 700 }, { width: 800, height: 600 })).toEqual({ left: 0, top: 0 });
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, handlers());
    const panel = el.querySelector<HTMLElement>(".modal__panel")!;
    Object.assign(window, { innerWidth: 800, innerHeight: 600 });
    Object.assign(panel.style, { left: "560px", top: "300px", width: "480px", height: "360px" });
    fitAnchoredPanel(panel);
    expect(panel.style.left).toBe("560px");
    el.classList.add("modal--anchored");
    fitAnchoredPanel(panel);
    expect([panel.style.left, panel.style.top, panel.style.width, panel.style.height, panel.style.maxWidth]).toEqual(["312px", "232px", "480px", "360px", "calc(100vw - 320px)"]);
  });

  it("pins the panel where it is, keeping the first position", () => {
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, handlers());
    const panel = el.querySelector<HTMLElement>(".modal__panel")!;
    el.getBoundingClientRect = () => rect(0, 0, 1280, 820);
    panel.getBoundingClientRect = () => rect(240, 110, 800, 600);
    anchorPanel(el, panel);
    expect([panel.style.left, panel.style.top, panel.style.maxWidth]).toEqual(["240px", "110px", "calc(100vw - 248px)"]);
    panel.getBoundingClientRect = () => rect(300, 200, 700, 500);
    anchorPanel(el, panel);
    expect(panel.style.left).toBe("240px");
  });

  it("opens at the size last dragged to, and at the stylesheet's size when none is stored", () => {
    localStorage.removeItem(PANEL_SIZE_KEY);
    const plain = renderModal({ card: base, turns: [], status: null, draft: "" }, handlers()).querySelector<HTMLElement>(".modal__panel")!;
    expect(plain.style.width).toBe("");
    localStorage.setItem(PANEL_SIZE_KEY, JSON.stringify({ width: 1000, height: 800 }));
    const sized = renderModal({ card: base, turns: [], status: null, draft: "" }, handlers()).querySelector<HTMLElement>(".modal__panel")!;
    expect(sized.style.width).toBe("1000px");
    expect(sized.style.height).toBe("800px");
    localStorage.removeItem(PANEL_SIZE_KEY);
  });
});

describe("renderModal", () => {
  it("shows header, turns with kind classes and a composer", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns, status: null, draft: "" }, h);
    expect(el.querySelector(".modal__title")?.textContent).toBe("eye-1");
    expect(el.querySelector(".modal__project")?.textContent).toBe("eye");
    expect(el.querySelector(".modal__state")?.textContent).toBe("Idle");
    const kinds = [...el.querySelectorAll(".turn")].map((t) => t.className);
    expect(kinds).toEqual(["turn turn--user", "turn turn--assistant", "turn turn--tool", "turn turn--peer"]);
    expect([...el.querySelectorAll(".turn__who")].map((w) => w.textContent)).toEqual(["You", "Claude", "From another session"]);
    expect(el.querySelector("textarea")).not.toBeNull();
    expect(el.querySelector(".modal__banner")).toBeNull();
  });

  it("shows the controls the agent has", () => {
    const h = handlers();
    const codex = renderModal({ card: { ...base, harness: "codex", hasInbox: false, context: { used: 80_000, window: 100_000, percent: 80 } }, turns: [], status: null, draft: "" }, h);
    expect(codex.querySelector("button[data-action=compact]")).not.toBeNull();
    expect(codex.querySelector("select[name=model]")).toBeNull();
    expect(codex.querySelector("button[data-action=cycle-mode]")).toBeNull();
    const grok = renderModal({ card: { ...base, harness: "grok", hasInbox: false }, turns: [], status: null, draft: "", agent: { harness: "grok", models: [{ id: "grok-4.7", label: "grok-4.7", efforts: [] }], efforts: [], modes: [] } }, h);
    expect([...grok.querySelectorAll<HTMLOptionElement>("select[name=model] option")].map((o) => o.value)).toEqual(["", "grok-4.7"]);
    expect(grok.querySelector("select[name=effort]")).toBeNull();
    expect(grok.querySelector("button[data-action=cycle-mode]")).not.toBeNull();
  });

  it("offers an OpenCode card the efforts of its own model", () => {
    const h = handlers();
    const agent = { harness: "opencode" as const, models: [{ id: "opencode/fledge-alpha-free", label: "Fledge", efforts: ["low", "max"] }, { id: "mlx/qwen", label: "Qwen", efforts: [] }], efforts: [], modes: [] };
    const el = renderModal({ card: { ...base, harness: "opencode", hasInbox: false, model: "opencode/fledge-alpha-free" }, turns: [], status: null, draft: "", agent }, h);
    expect([...el.querySelectorAll<HTMLOptionElement>("select[name=effort] option")].map((o) => o.value)).toEqual(["", "low", "max"]);
    const bare = renderModal({ card: { ...base, harness: "opencode", hasInbox: false, model: "mlx/qwen" }, turns: [], status: null, draft: "", agent }, h);
    expect(bare.querySelector("select[name=effort]")).toBeNull();
  });

  it("names OpenCode's mode cycle as the build and plan agents", () => {
    const h = handlers();
    const el = renderModal({ card: { ...base, harness: "opencode", hasInbox: false }, turns: [], status: null, draft: "" }, h);
    const cycle = el.querySelector<HTMLButtonElement>("button[data-action=cycle-mode]")!;
    expect(cycle.title).toContain("build and plan");
    const grok = renderModal({ card: { ...base, harness: "grok", hasInbox: false }, turns: [], status: null, draft: "" }, h);
    expect(grok.querySelector<HTMLButtonElement>("button[data-action=cycle-mode]")!.title).toContain("Shift+Tab");
  });

  it("brings the pickers in on a repaint, but keeps an unapplied choice", () => {
    const h = handlers();
    const grok = { ...base, harness: "grok" as const, hasInbox: false };
    const agent = { harness: "grok" as const, models: [{ id: "grok-4.7", label: "grok-4.7", efforts: [] }], efforts: [], modes: [] };
    // First paint: the agent list has not landed, so only the mode cycle shows.
    const root = renderModal({ card: grok, turns: [], status: null, draft: "" }, h);
    expect(root.querySelector("select[name=model]")).toBeNull();
    patchModal(root, renderModal({ card: grok, turns: [], status: null, draft: "", agent }, h));
    const model = root.querySelector<HTMLSelectElement>("select[name=model]");
    expect(model, "the picker arrives with the agent list").not.toBeNull();
    // A choice waiting for Apply survives a background repaint.
    model!.value = "grok-4.7";
    patchModal(root, renderModal({ card: grok, turns: [], status: null, draft: "", agent }, h));
    expect(root.querySelector<HTMLSelectElement>("select[name=model]")!.value).toBe("grok-4.7");
  });

  it("keeps controls Codex lacks off its session but still lets you type a reply", () => {
    const h = handlers();
    const codex = { ...base, harness: "codex" as const, hasInbox: false, context: { used: 160_000, window: 200_000, percent: 80 } };
    const el = renderModal({ card: codex, turns: [], status: null, draft: "" }, h);
    expect(el.querySelector(".modal__tweaks")).toBeNull();
    expect(el.querySelector("button[data-action=compact]")).not.toBeNull();
    expect(el.querySelector(".usage__meter[data-kind=ctx] .usage__pct")?.textContent).toBe("80%");
    const ta = el.querySelector<HTMLTextAreaElement>("textarea")!;
    expect(ta.placeholder).toContain("typed into its terminal as one line, so /commands run there)");
    expect(ta.placeholder).not.toContain("!shell");
    expect(el.querySelector(".modal__noinbox")).toBeNull();
    ta.value = "carry on";
    el.querySelector<HTMLButtonElement>("button[data-action=send]")!.click();
    expect(h.onSend).toHaveBeenCalledWith("carry on");
  });

  it("renames on Enter from the clicked title, and only when the name changed", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    document.body.replaceChildren(el);
    const title = el.querySelector<HTMLElement>(".modal__title")!;
    expect(title.title).toContain("rename");
    title.click();
    const input = el.querySelector<HTMLInputElement>("input.modal__title-input")!;
    expect(input.value).toBe("eye-1");
    expect(document.activeElement).toBe(input);
    input.value = "  eye-1  ";
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(h.onRename).not.toHaveBeenCalled();
    expect(el.querySelector(".modal__title")?.textContent).toBe("eye-1");
    el.querySelector<HTMLElement>(".modal__title")!.click();
    const again = el.querySelector<HTMLInputElement>("input.modal__title-input")!;
    again.value = "board work";
    again.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(h.onRename).toHaveBeenCalledWith("board work");
    expect(el.querySelector("input.modal__title-input")).toBeNull();
  });

  it("offers the rename from the title on Codex, Antigravity and Grok sessions too", () => {
    for (const harness of ["codex", "antigravity", "grok"] as const) {
      const h = handlers();
      const el = renderModal({ card: { ...base, harness, hasInbox: false }, turns: [], status: null, draft: "" }, h);
      document.body.replaceChildren(el);
      const title = el.querySelector<HTMLElement>(".modal__title")!;
      expect(title.title).toContain("rename");
      expect(title.tabIndex).toBe(0);
      title.click();
      const input = el.querySelector<HTMLInputElement>("input.modal__title-input")!;
      input.value = "board work";
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      expect(h.onRename).toHaveBeenCalledWith("board work");
    }
  });

  it("cancels the rename on Escape without closing the modal", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    document.body.replaceChildren(el);
    const closeOnEscape = vi.fn((e: KeyboardEvent) => { if (e.key === "Escape") h.onClose(); });
    document.addEventListener("keydown", closeOnEscape);
    el.querySelector<HTMLElement>(".modal__title")!.click();
    const input = el.querySelector<HTMLInputElement>("input.modal__title-input")!;
    input.value = "changed";
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    document.removeEventListener("keydown", closeOnEscape);
    expect(h.onClose).not.toHaveBeenCalled();
    expect(h.onRename).not.toHaveBeenCalled();
    expect(el.querySelector(".modal__title")?.textContent).toBe("eye-1");
  });

  it("shows context usage in the header and a Compact button from 75 percent", () => {
    const h = handlers();
    const low = renderModal({ card: { ...base, context: { used: 20_000, window: 200_000, percent: 10 } }, turns: [], status: null, draft: "" }, h);
    expect(low.querySelector(".usage__meter[data-kind=ctx] .usage__pct")?.textContent).toBe("10%");
    expect(low.querySelector("button[data-action=compact]")).toBeNull();
    const high = renderModal({ card: { ...base, context: { used: 160_000, window: 200_000, percent: 80 } }, turns: [], status: null, draft: "" }, h);
    const btn = high.querySelector<HTMLButtonElement>(".modal__head button[data-action=compact]")!;
    btn.click();
    expect(h.onCompact).toHaveBeenCalledTimes(1);
  });

  it("the_header_carries_the_usage_strip_and_no_context_span", () => {
    const h = handlers();
    const card: Card = { ...base, pr: { number: 12, url: "u", state: "open" }, context: { used: 160_000, window: 200_000, percent: 80 }, limits: { account: null, windows: [{ label: "5h", percent: 20, resetsAt: null }], readAt: 0 } };
    const el = renderModal({ card, turns: [], status: null, draft: "" }, h);
    const strip = el.querySelector<HTMLElement>(".modal__titles .usage");
    expect(strip).not.toBeNull();
    expect(el.querySelector(".modal__context")).toBeNull();
    expect(strip!.previousElementSibling?.matches("[data-action=pr]"), "after the PR button").toBe(true);
    expect(strip!.querySelectorAll(".usage__meter")).toHaveLength(2);
    expect(el.querySelector(".modal__head button[data-action=compact]")).not.toBeNull();
    expect(renderModal({ card: { ...base, context: { used: 20_000, window: 200_000, percent: 10 } }, turns: [], status: null, draft: "" }, h).querySelector("button[data-action=compact]")).toBeNull();
  });

  it("the_strip_stage_follows_the_heads_width", () => {
    const card: Card = { ...base, context: { used: 124_000, window: 200_000, percent: 62 }, limits: { account: null, windows: [{ label: "5h", percent: 78, resetsAt: null }], readAt: 0 } };
    const root = renderModal({ card, turns: [], status: null, draft: "" }, handlers());
    const head = root.querySelector<HTMLElement>(".modal__head")!;
    applyStage(root, 1200);
    expect(head.dataset.stage).toBe("wide");
    expect(root.querySelector<HTMLElement>(".usage")!.dataset.stage).toBe("wide");
    applyStage(root, 900);
    expect(head.dataset.stage).toBe("medium");
    expect(root.querySelector(".usage")!.className).toBe("usage usage--medium");
    applyStage(root, 700);
    expect(head.dataset.stage).toBe("narrow");
    expect(root.querySelector(".usage__bar")).toBeNull();
    applyStage(root, 500);
    expect(head.dataset.stage).toBe("chip");
    expect(root.querySelector(".usage__chip")?.textContent).toBe("62% · 78%");
    // A repaint keeps the stage the head is at.
    patchModal(root, renderModal({ card: { ...card, context: { used: 140_000, window: 200_000, percent: 70 } }, turns: [], status: null, draft: "" }, handlers()));
    expect(root.querySelectorAll(".usage")).toHaveLength(1);
    expect(root.querySelector(".usage__chip")?.textContent).toBe("70% · 78%");
  });

  it("hides the header's status line at narrow and chip, and the folder at chip", () => {
    expect(styles).toContain('.modal__head[data-stage="narrow"] .modal__statusline--head, .modal__head[data-stage="chip"] .modal__statusline--head { display: none; }');
    expect(styles).toContain('.modal__head[data-stage="chip"] .modal__project { display: none; }');
    expect(styles).toContain(".usage { flex: none; display: flex;");
  });

  it("keeps every item in the head on one line: the status line gives way first, then the title", () => {
    // Every item of the titles row keeps its width and its line; the strip never shrinks below its meters.
    expect(styles).toContain(".modal__titles > * { flex: none; white-space: nowrap; }");
    expect(styles).toContain(".usage { flex: none; display: flex;");
    expect(styles).toContain(".modal__project { color: var(--muted); font-size: 12px; white-space: nowrap; }");
    expect(styles).toContain(".modal__state { flex: none; white-space: nowrap;");
    // The two that may shrink, the status line far sooner than the title, each ending in an ellipsis.
    expect(styles).toContain(".modal__titles > .modal__title { flex: 0 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; }");
    expect(styles).toContain(".modal__titles > .modal__statusline--head { flex: 0 100000 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }");
    expect(styles).toContain(".modal__titles { flex: 1; display: flex; align-items: baseline; gap: 8px; min-width: 0; }");
    // The rules land on what renderModal draws.
    const el = renderModal({ card: { ...base, model: "claude-opus-5", branch: "feat/x" }, turns: [], status: null, draft: "" }, handlers());
    const titles = el.querySelector(".modal__head > .modal__titles")!;
    for (const sel of [".modal__title", ".modal__project", ".usage", ".modal__statusline--head"]) expect(titles.querySelector(`:scope > ${sel}`), sel).not.toBeNull();
  });

  it("keeps limit bars blue until 75 percent turns them amber, and context bars gold", () => {
    expect(styles).toContain('.usage__meter[data-kind="limit"] .usage__fill { background: var(--working); }');
    expect(styles).toContain('.usage__meter[data-band="high"] .usage__fill { background: var(--awaiting); }');
    // No mid-band fill colour: a limit at 50-74% must not look like the context bar.
    expect(styles).not.toMatch(/data-band="mid"\][^{]*\.usage__fill/);
  });

  it("mutes a window whose reset passed, in both looks", () => {
    // Four classes and attributes: above the terminal look's `.modal--terminal .usage__pct` colours.
    expect(styles).toContain('.usage .usage__meter[data-reset="passed"] .usage__pct { color: inherit; font-style: italic; }');
  });

  it("tells the user to reply when the session asked in prose", () => {
    const h = handlers();
    const asked = { ...base, state: "awaiting" as const, awaiting: { kind: "text" as const, detail: "Create it as drafted?", questions: [] } };
    const el = renderModal({ card: asked, turns: [], status: null, draft: "" }, h);
    expect(el.querySelector(".modal__banner")?.textContent).toContain("asked you something");
    expect(el.querySelector(".modal__banner")?.textContent).toContain("Create it as drafted?");
    expect(el.querySelector(".options")).toBeNull();
    expect(el.querySelector("textarea")).not.toBeNull();
  });

  it("shows the harness in the header and a PR button only when a PR is known", () => {
    const h = handlers();
    const none = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    const badge = none.querySelector<HTMLImageElement>("img.modal__harness")!;
    expect(badge.alt).toBe("Claude Code");
    expect(badge.src).toContain("claude-code");
    expect(none.querySelector("button[data-action=pr]")).toBeNull();
    const el = renderModal({ card: { ...base, pr: { number: 12, url: "https://github.com/o/r/pull/12", state: "draft" } }, turns: [], status: null, draft: "" }, h);
    const btn = el.querySelector<HTMLButtonElement>(".modal__head button[data-action=pr]")!;
    expect(btn.textContent).toBe("PR #12");
    expect(btn.title).toContain("draft");
    btn.click();
    expect(h.onOpenPr).toHaveBeenCalledTimes(1);
  });

  it("applies only the model and effort that were changed, then resets the pickers", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    const row = el.querySelector(".modal__tweaks")!;
    expect(row).not.toBeNull();
    const model = row.querySelector<HTMLSelectElement>("select[name=model]")!;
    const effort = row.querySelector<HTMLSelectElement>("select[name=effort]")!;
    expect(model.options[0].textContent).toBe("Model");
    expect(effort.options[0].textContent).toBe("Effort");
    expect([...model.options].map((o) => o.value)).toEqual(["", "fable", "opus", "sonnet", "haiku"]);
    expect([...effort.options].map((o) => o.value)).toEqual(["", "low", "medium", "high", "xhigh", "max"]);
    const apply = row.querySelector<HTMLButtonElement>("button[data-action=apply]")!;
    expect(apply.disabled).toBe(true);
    apply.click();
    expect(h.onSetOption).not.toHaveBeenCalled();
    effort.value = "xhigh";
    effort.dispatchEvent(new Event("change"));
    expect(apply.disabled).toBe(false);
    apply.click();
    expect(h.onSetOption).toHaveBeenCalledTimes(1);
    expect(h.onSetOption).toHaveBeenCalledWith("effort", "xhigh");
    expect(effort.value).toBe("");
    expect(apply.disabled).toBe(true);
    model.value = "opus";
    effort.value = "low";
    model.dispatchEvent(new Event("change"));
    apply.click();
    expect(h.onSetOption).toHaveBeenNthCalledWith(2, "model", "opus");
    expect(h.onSetOption).toHaveBeenNthCalledWith(3, "effort", "low");
  });

  it("has a Cycle mode button that sends Shift+Tab", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    const btn = el.querySelector<HTMLButtonElement>("button[data-action=cycle-mode]")!;
    expect(btn.textContent).toContain("mode");
    expect(btn.title).toContain("Shift+Tab");
    btn.click();
    expect(h.onCycleMode).toHaveBeenCalledTimes(1);
  });

  it("offers the tweaks row and the composer when a Claude session has no inbox", () => {
    const el = renderModal({ card: { ...base, hasInbox: false }, turns: [], status: null, draft: "" }, handlers());
    expect(el.querySelector(".modal__tweaks")).not.toBeNull();
    expect(el.querySelector("textarea")).not.toBeNull();
    expect(el.querySelector(".modal__noinbox")).toBeNull();
  });

  it("renders turns as markdown and routes link clicks to the handler without navigating", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns: [{ kind: "assistant", text: "Done: **all green**. See [the PR](https://github.com/o/r/pull/1)." }, { kind: "tool", text: "Bash: **not** markdown" }], status: null, draft: "" }, h);
    const turns = el.querySelectorAll(".turn__text");
    expect(turns[0].querySelector("strong")?.textContent).toBe("all green");
    expect(turns[1].querySelector("strong")).toBeNull();
    const link = turns[0].querySelector<HTMLAnchorElement>("a")!;
    const ev = new MouseEvent("click", { bubbles: true, cancelable: true });
    link.dispatchEvent(ev);
    expect(h.onOpenLink).toHaveBeenCalledWith("https://github.com/o/r/pull/1");
    expect(ev.defaultPrevented).toBe(true);
  });

  it("sends the composed message with attachment lines and clears the chips", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "", attachments: [{ name: "shot.png", path: "/a/shot.png" }] }, h);
    expect(el.querySelector(".modal__chips .chip__name")?.textContent).toBe("shot.png");
    const ta = el.querySelector<HTMLTextAreaElement>("textarea")!;
    ta.value = "see this";
    el.querySelector<HTMLButtonElement>("button[data-action=send]")!.click();
    expect(h.onSend).toHaveBeenCalledWith("see this\n\nAttached file: /a/shot.png");
    // Attachments alone are enough to send.
    const only = renderModal({ card: base, turns: [], status: null, draft: "", attachments: [{ name: "a.png", path: "/a.png" }] }, h);
    only.querySelector<HTMLButtonElement>("button[data-action=send]")!.click();
    expect(h.onSend).toHaveBeenLastCalledWith("Attached file: /a.png");
  });

  it("sends a file attached after the composer was drawn, since a repaint keeps the composer", () => {
    const h = handlers();
    const m: Parameters<typeof renderModal>[0] = { card: base, turns: [], status: null, draft: "" };
    const root = renderModal(m, h);
    m.attachments = [{ name: "shot.png", path: "/a/shot.png" }];
    patchModal(root, renderModal(m, h));
    expect(root.querySelector(".modal__chips .chip__name")?.textContent).toBe("shot.png");
    root.querySelector<HTMLTextAreaElement>("textarea")!.value = "see this";
    root.querySelector<HTMLButtonElement>("button[data-action=send]")!.click();
    expect(h.onSend).toHaveBeenCalledWith("see this\n\nAttached file: /a/shot.png");
  });

  it("sends trimmed text on button click and on Enter, never blank", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    const ta = el.querySelector<HTMLTextAreaElement>("textarea")!;
    ta.value = "   ";
    el.querySelector<HTMLButtonElement>("button[data-action=send]")!.click();
    expect(h.onSend).not.toHaveBeenCalled();
    ta.value = "  reply please  ";
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(h.onSend).toHaveBeenCalledWith("reply please");
  });

  it("sends and adds lines with the keys chosen in Settings, and names the send key in the hint", () => {
    setKeyBindings({ send: ["Meta+Enter"], newLine: ["Enter"] });
    try {
      const h = handlers();
      const el = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
      const ta = el.querySelector<HTMLTextAreaElement>("textarea")!;
      expect(ta.placeholder).toContain(`(${comboLabel("Meta+Enter")} to send`);
      ta.value = "two";
      ta.setSelectionRange(0, 0);
      ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      expect(h.onSend).not.toHaveBeenCalled();
      expect(ta.value).toBe("\ntwo");
      ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true, cancelable: true }));
      expect(h.onSend).toHaveBeenCalledWith("two");
    } finally {
      setKeyBindings(defaultKeys());
    }
  });

  it("types a /command into the terminal instead of sending it as a reply, for agents that take them", () => {
    expect(isTerminalCommand("  /review")).toBe(true);
    expect(isTerminalCommand("review /x")).toBe(false);
    expect(isTerminalCommand("")).toBe(false);
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "", attachments: [{ name: "a.png", path: "/a.png" }] }, h);
    expect(el.querySelector<HTMLTextAreaElement>("textarea")!.placeholder).toContain("/command");
    const ta = el.querySelector<HTMLTextAreaElement>("textarea")!;
    ta.value = " /review the diff ";
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(h.onCommand).toHaveBeenCalledWith("/review the diff");
    expect(h.onSend).not.toHaveBeenCalled();
    // Codex takes / commands too, but has no ! shell lines: one is just a reply there.
    const codex = renderModal({ card: { ...base, harness: "codex", hasInbox: false }, turns: [], status: null, draft: "" }, h);
    const ta2 = codex.querySelector<HTMLTextAreaElement>("textarea")!;
    ta2.value = "/status";
    codex.querySelector<HTMLButtonElement>("button[data-action=send]")!.click();
    expect(h.onCommand).toHaveBeenLastCalledWith("/status");
    ta2.value = "!ls";
    codex.querySelector<HTMLButtonElement>("button[data-action=send]")!.click();
    expect(h.onSend).toHaveBeenCalledWith("!ls");
    expect(h.onCommand).toHaveBeenCalledTimes(2);
  });

  it("types a !shell line into the terminal the same way", () => {
    expect(isTerminalCommand(" !ls")).toBe(true);
    expect(isTerminalCommand("ls !")).toBe(false);
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    expect(el.querySelector<HTMLTextAreaElement>("textarea")!.placeholder).toContain("!shell");
    const ta = el.querySelector<HTMLTextAreaElement>("textarea")!;
    ta.value = " ! git status ";
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(h.onCommand).toHaveBeenCalledWith("! git status");
    expect(h.onSend).not.toHaveBeenCalled();
  });

  it("shows the awaiting banner with a terminal button, and a status line", () => {
    const h = handlers();
    const el = renderModal({ card: { ...base, state: "awaiting", awaiting: { kind: "permission", detail: "Bash: rm", questions: [] } }, turns: [], status: { ok: false, text: "boom" }, draft: "" }, h);
    expect(el.querySelector(".modal__banner")?.textContent).toContain("waiting for a decision");
    el.querySelector<HTMLButtonElement>(".modal__banner button")!.click();
    expect(h.onTerminal).toHaveBeenCalled();
    expect(el.querySelector(".modal__status")?.textContent).toBe("boom");
    expect(el.querySelector(".modal__status")?.className).toContain("modal__status--error");
  });

  it("keeps only its text and Open terminal in the banner while the menu asks", () => {
    const h = handlers();
    const q = renderModal({ card: asking, turns: [], status: null, draft: "", next: 0 }, h, 6000);
    const banner = q.querySelector(".modal__banner")!;
    expect(banner.textContent).toBe("This session is asking. Answer below or in its terminal.Open terminal");
    expect(banner.querySelector(".options, .menu")).toBeNull();
    expect([...banner.querySelectorAll("button")].map((b) => b.textContent)).toEqual(["Open terminal"]);
  });

  it("the menu is disabled inside the open delay", () => {
    const h = handlers();
    const early = renderModal({ card: asking, turns: [], status: null, draft: "", next: 0 }, h, 5500);
    expect(early.querySelector(".menu__rows")?.getAttribute("aria-disabled")).toBe("true");
    early.querySelector<HTMLElement>(".menu [data-row='0']")!.click();
    expect(h.onPick).not.toHaveBeenCalled();
    const q = renderModal({ card: asking, turns: [], status: null, draft: "", next: 0 }, h, 6000);
    expect(q.querySelector(".menu__rows")?.getAttribute("aria-disabled")).toBeNull();
    expect(q.querySelector(".menu .menu__desc")?.textContent).toBe("desc a");
    const row = q.querySelector<HTMLElement>(".menu [data-row='0']")!;
    row.click();
    expect(h.onPick).toHaveBeenCalledWith(0, { kind: "option", index: 0 }, row);
  });

  it("menu replaces the composer while asking", () => {
    const h = handlers();
    const q = renderModal({ card: asking, turns: [], status: null, draft: "", next: 0 }, h, 6000);
    expect(q.querySelector(".menu")).not.toBeNull();
    expect(q.querySelector(".modal__input")).toBeNull();
    expect(q.querySelector(".modal__composer")).toBeNull();
    // Once every question has an answer the composer is back until the card moves on.
    const answered = renderModal({ card: asking, turns: [], status: null, draft: "", next: 1 }, h, 6000);
    expect(answered.querySelector(".menu")).toBeNull();
    expect(answered.querySelector(".modal__input")).not.toBeNull();
  });

  it("hands the phone's menu no keyboard hints", () => {
    const h = handlers();
    expect(renderModal({ card: asking, turns: [], status: null, draft: "", next: 0 }, h, 6000).querySelector(".menu__hints")).not.toBeNull();
    setComposerOptions({ keyboardHints: false });
    expect(renderModal({ card: asking, turns: [], status: null, draft: "", next: 0 }, h, 6000).querySelector(".menu__hints")).toBeNull();
    setComposerOptions({ keyboardHints: true });
  });

  it("shows a multi-select question read-only and sends the user to the terminal", () => {
    const multi = { ...asking, awaiting: { kind: "question" as const, detail: "Q?", questions: [{ question: "Q?", header: "H", multiSelect: true, options: [{ label: "A", description: "" }] }] } };
    const el = renderModal({ card: multi, turns: [], status: null, draft: "", next: 0 }, handlers(), 6000);
    expect(el.querySelector(".menu__note")?.textContent).toBe("Multi-select: answer in the terminal");
    expect(el.querySelector(".menu [data-action=pick]")).toBeNull();
    expect(el.querySelector(".modal__banner")?.textContent).toContain("Answer it there");
  });

  it("Stop shows only when canStop, and sits next to Send", () => {
    const h = handlers();
    const working = { ...base, state: "working" as const, stateSince: 5000 };
    const el = renderModal({ card: working, turns: [], status: null, draft: "" }, h, 6000);
    const stop = el.querySelector<HTMLButtonElement>(".modal__composer button[data-action=stop]")!;
    expect(stop.textContent).toBe("Stop");
    expect(stop.nextElementSibling?.getAttribute("data-action")).toBe("send");
    expect(stop.disabled).toBe(false);
    stop.click();
    expect(h.onStop).toHaveBeenCalledWith(stop);
    expect(renderModal({ card: working, turns: [], status: null, draft: "" }, h, 5500).querySelector<HTMLButtonElement>("button[data-action=stop]")!.disabled).toBe(true);
    expect(renderModal({ card: base, turns: [], status: null, draft: "" }, h, 6000).querySelector("button[data-action=stop]")).toBeNull();
    expect(renderModal({ card: { ...working, shell: true }, turns: [], status: null, draft: "" }, h, 6000).querySelector("button[data-action=stop]")).toBeNull();
    expect(renderModal({ card: { ...working, harness: "kiro" }, turns: [], status: null, draft: "" }, h, 6000).querySelector("button[data-action=stop]")).toBeNull();
    // While asking, Stop sits in the menu's foot.
    const q = renderModal({ card: asking, turns: [], status: null, draft: "", next: 0 }, h, 6000);
    expect(q.querySelector(".menu__foot button[data-action=stop]")).not.toBeNull();
  });

  it("shows the machine in the header and drops the terminal button from the awaiting banner for a remote card", () => {
    const h = handlers();
    const remote = { ...base, machine: "laptop", state: "awaiting" as const, awaiting: { kind: "permission" as const, detail: "Bash: rm", questions: [] } };
    const el = renderModal({ card: remote, turns: [], status: null, draft: "" }, h);
    expect(el.querySelector(".card__remote")?.getAttribute("title")).toBe("Runs on laptop");
    expect(el.querySelector(".modal__banner button")).toBeNull();
    const local = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    expect(local.querySelector(".card__remote")).toBeNull();
  });

  it("names the machine's address in the header glyph's tooltip, once", () => {
    const h = handlers();
    const plain = renderModal({ card: { ...base, machine: "Gnowee", machineAddress: "192.168.55.70" }, turns: [], status: null, draft: "" }, h);
    expect(plain.querySelector(".card__remote")?.getAttribute("title")).toBe("Runs on Gnowee, 192.168.55.70");
    const twin = renderModal({ card: { ...base, machine: "Gnowee (192.168.55.70)", machineAddress: "192.168.55.70" }, turns: [], status: null, draft: "" }, h);
    expect(twin.querySelector(".card__remote")?.getAttribute("title")).toBe("Runs on Gnowee (192.168.55.70)");
    const win = renderModal({ card: { ...base, machine: "Gnowee", machineAddress: "192.168.55.70", machinePlatform: "windows" }, turns: [], status: null, draft: "" }, h);
    expect(win.querySelector(".card__remote")?.getAttribute("title")).toBe("Runs on Gnowee (Windows), 192.168.55.70");
  });

  it("names the tmux session in the header glyph's tooltip", () => {
    const h = handlers();
    const el = renderModal(
      { card: { ...base, machine: "laptop", machineAddress: "10.0.0.9", machinePlatform: "macos", terminal: "maya-1a2b3c4d" }, turns: [], status: null, draft: "" },
      h,
    );
    expect(el.querySelector(".card__remote")?.getAttribute("title")).toBe("Runs on laptop (macOS), 10.0.0.9; attach: tmux attach -t maya-1a2b3c4d");
  });

  it("never suggests the terminal in the awaiting banner for a remote card, naming the machine instead", () => {
    const h = handlers();
    const permission = renderModal(
      { card: { ...base, machine: "laptop", state: "awaiting" as const, awaiting: { kind: "permission" as const, detail: "Bash: rm", questions: [] } }, turns: [], status: null, draft: "" },
      h,
    );
    expect(permission.querySelector(".modal__banner")?.textContent).not.toContain("terminal");
    expect(permission.querySelector(".modal__banner")?.textContent).toContain("waiting for a decision on laptop");

    const prose = renderModal(
      {
        card: { ...base, machine: "laptop", state: "awaiting" as const, awaiting: { kind: "text" as const, detail: "Create it as drafted?", questions: [] } },
        turns: [],
        status: null,
        draft: "",
      },
      h,
    );
    expect(prose.querySelector(".modal__banner")?.textContent).not.toContain("terminal");
    expect(prose.querySelector(".modal__banner")?.textContent).toContain("Reply below.");

    const question = renderModal({ card: { ...asking, machine: "laptop" }, turns: [], status: null, draft: "", next: 0 }, h, 6000);
    expect(question.querySelector(".modal__banner")?.textContent).not.toContain("terminal");
    expect(question.querySelector(".modal__banner")?.textContent).toBe("This session is asking. Answer below.");
  });

  it("closes on backdrop click and on the close button", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    el.querySelector<HTMLElement>(".modal__backdrop")!.click();
    el.querySelector<HTMLButtonElement>("button[data-action=close]")!.click();
    expect(h.onClose).toHaveBeenCalledTimes(2);
  });
});

describe("an attachment's size", () => {
  it("is refused over the backend's 20 MB cap before the file is read, with the backend's own words", () => {
    expect(attachmentTooLarge(0)).toBeNull();
    expect(attachmentTooLarge(MAX_ATTACHMENT_BYTES)).toBeNull();
    expect(attachmentTooLarge(MAX_ATTACHMENT_BYTES + 1)).toBe("The file is too large (over 20 MB).");
  });
});

describe("the Attach button", () => {
  it("is absent by default and, when on, hands picked files to onPasteFiles", () => {
    const h = handlers();
    const model = { card: base, turns: [], status: null, draft: "" };
    expect(renderModal(model, h).querySelector("button[data-action=attach]")).toBeNull();
    setComposerOptions({ attachButton: true });
    try {
      const el = renderModal(model, h);
      const attach = el.querySelector<HTMLButtonElement>("button[data-action=attach]")!;
      const pick = el.querySelector<HTMLInputElement>("input[type=file]")!;
      expect(pick.multiple).toBe(true);
      expect(pick.hidden).toBe(true);
      const file = new File(["hi"], "note.txt", { type: "text/plain" });
      Object.defineProperty(pick, "files", { value: [file] });
      pick.dispatchEvent(new Event("change"));
      expect(h.onPasteFiles).toHaveBeenCalledWith([{ name: "note.txt", file }]);
      expect(attach.textContent).toBe("Attach");
    } finally {
      setComposerOptions({ attachButton: false });
    }
  });
});

describe("replyStatus", () => {
  it("says whether a reply went in as the user or as another session", () => {
    expect(replyStatus("typed")).toEqual({ ok: true, text: "Sent as you" });
    expect(replyStatus("inbox")).toEqual({ ok: true, warn: true, text: "Sent as a message from another session: it can't approve anything" });
    expect(replyStatus(null)).toEqual({ ok: true, text: "Delivered" });
  });
});

describe("renderModal notices and statuses", () => {
  it("shows a notice as a line with no speaker", () => {
    const el = renderModal({ card: base, turns: [{ kind: "tool", text: "Interrupted", notice: true }], status: null, draft: "" }, handlers());
    const t = el.querySelector(".turn--notice")!;
    expect(t.querySelector(".turn__who")).toBeNull();
    expect(t.textContent).toBe("Interrupted");
  });

  it("colours an inbox fallback as a warning", () => {
    const el = renderModal({ card: base, turns: [], status: replyStatus("inbox"), draft: "" }, handlers());
    expect(el.querySelector(".modal__status")?.className).toBe("modal__status modal__status--warn");
  });

  it("tells the user to answer a pending decision before replying", () => {
    const awaiting = { ...base, state: "awaiting" as const, awaiting: { kind: "permission" as const, detail: "Bash: git push", questions: [] } };
    const el = renderModal({ card: awaiting, turns: [], status: null, draft: "" }, handlers());
    expect(el.querySelector(".modal__banner")?.textContent).toContain("Answer it there before replying.");
  });
});

describe("viewer theme", () => {
  it("draws the terminal look's markers and prompt, the chat look without the class", () => {
    const term = renderModal({ card: base, turns, status: null, draft: "", theme: "terminal" }, handlers());
    expect(term.classList.contains("modal--terminal")).toBe(true);
    expect([...term.querySelectorAll(".turn")].map((t) => t.querySelector(".turn__mark")?.textContent ?? "")).toEqual([">", "⏺", "⏺", ""]);
    expect(term.querySelector(".modal__prompt")?.textContent).toBe(">");
    const chat = renderModal({ card: base, turns, status: null, draft: "" }, handlers());
    expect(chat.classList.contains("modal--terminal")).toBe(false);
  });

  it("hides the markers in the chat look and draws the terminal look's colours and font", () => {
    expect(styles).toContain(".turn__mark, .modal__prompt { display: none; }");
    expect(styles).toContain(".modal--terminal .turn__mark { display: inline;");
    expect(styles).toContain(".modal--terminal .modal__panel { background: #0a0a0a; color: #e8e2d4; }");
    expect(styles).toContain("ui-monospace, SFMono-Regular, Menlo, monospace");
  });

  it("gives the terminal look's user lines a lighter row, as the CLI does, in both themes", () => {
    expect(styles).toContain(".modal--terminal .turn--user { background: #1a1915; padding: 4px 6px; margin: 0 -6px; border-radius: 4px; }");
    expect(styles).toContain(':root[data-theme="light"] .modal--terminal .turn--user { background: color-mix(in srgb, var(--bg) 70%, transparent); }');
  });

  it("keeps a multi-line tool call whole in one .turn__text, in both looks", () => {
    const text = "Bash(cat <<EOF\nline two\nEOF)";
    for (const theme of ["terminal", undefined] as const) {
      const el = renderModal({ card: base, turns: [{ kind: "tool", text }], status: null, draft: "", theme }, handlers());
      expect(el.querySelectorAll(".turn--tool .turn__text").length).toBe(1);
      expect(el.querySelector(".turn--tool .turn__text")?.textContent).toBe(text);
      expect(el.querySelector(".turn__result")).toBeNull();
    }
  });

  it("the header toggle asks for the other look, titled for what it switches to", () => {
    const h = { ...handlers(), onToggleTheme: vi.fn() };
    const chat = renderModal({ card: base, turns, status: null, draft: "" }, h);
    const toggle = chat.querySelector<HTMLButtonElement>("button[data-action=theme]")!;
    expect(toggle.title).toBe("Terminal look");
    toggle.click();
    expect(h.onToggleTheme).toHaveBeenCalled();
    expect(renderModal({ card: base, turns, status: null, draft: "", theme: "terminal" }, h).querySelector<HTMLButtonElement>("button[data-action=theme]")!.title).toBe("Chat look");
  });

  it("setViewerTheme redraws an open panel at once", () => {
    const host = document.createElement("div");
    host.id = "modal-host";
    document.body.append(host);
    host.append(renderModal({ card: base, turns, status: null, draft: "" }, handlers()));
    setViewerTheme("terminal");
    expect(host.querySelector(".modal")!.classList.contains("modal--terminal")).toBe(true);
    expect(host.querySelector<HTMLButtonElement>(".modal__theme")!.title).toBe("Chat look");
    setViewerTheme("chat");
    expect(host.querySelector(".modal")!.classList.contains("modal--terminal")).toBe(false);
    host.remove();
  });
});

describe("tool output in the viewer", () => {
  it("draws a tool call with its result in both looks, through the one renderer", () => {
    const t: Turn = { kind: "tool", text: "Bash(ls)", output: "a.rs\nb.rs", outputLines: 2 };
    for (const theme of ["terminal", undefined] as const) {
      const el = renderModal({ card: base, turns: [t], status: null, draft: "", theme }, handlers());
      expect(el.querySelector(".turn--tool .turn__row")?.textContent).toBe("▸ Bash · ls");
      expect(el.querySelector(".turn--tool .turn__text")?.textContent).toBe("Bash(ls)");
      expect(el.querySelector(".turn--tool .turn__out")?.textContent).toBe("a.rs\nb.rs");
    }
  });
});

describe("working and status lines", () => {
  const busy: Card = { ...base, state: "working", stateSince: 1_000, model: "claude-opus-5", branch: "feat/x", pr: { number: 12, url: "u", state: "open" }, context: { used: 84_000, window: 200_000, percent: 42 } };

  it("says how long the session has worked", () => {
    expect(workingFor(4_000)).toBe("4s");
    expect(workingFor(724_000)).toBe("12m 4s");
    expect(workingFor(3_725_000)).toBe("1h 2m");
    expect(workingFor(-5_000)).toBe("0s");
  });

  it("shows ✻ Working… while the session works, in both looks", () => {
    for (const theme of ["terminal", "chat"] as const) {
      const el = renderModal({ card: busy, turns, status: null, draft: "", theme }, handlers(), 1_000 + 724_000);
      expect(el.querySelector(".modal__history .modal__working")?.textContent).toBe("✻ Working… (12m 4s)");
    }
    expect(renderModal({ card: base, turns, status: null, draft: "" }, handlers()).querySelector(".modal__working")).toBeNull();
  });

  it("the_terminal_foot_shows_the_recorded_line_when_there_is_one", () => {
    const card: Card = { ...busy, statusLine: "\u001b[1mOpus\u001b[0m │ 62%" };
    expect(statusLine(card)).toBe(card.statusLine);
    const el = renderModal({ card, turns, status: null, draft: "", theme: "terminal" }, handlers(), 2_000);
    const foot = el.querySelector(".modal__composer + .modal__statusline--foot")!;
    expect(foot.textContent).toBe("Opus │ 62%");
    expect(foot.querySelector(".ansi-b")).not.toBeNull();
    expect(foot.classList.contains("modal__statusline--recorded")).toBe(true);
  });

  it("mayas_own_line_appends_limits_and_cost_without_a_recording", () => {
    expect(statusLine(busy)).toBe("claude-opus-5 · eye · feat/x · PR #12 · ctx 42%");
    expect(statusLine({ ...busy, pr: { number: 3, url: "u", state: "open", forge: "gitlab" } })).toBe("claude-opus-5 · eye · feat/x · MR !3 · ctx 42%");
    expect(statusLine(base)).toBe("eye");
    const card: Card = {
      ...busy,
      limits: { account: null, readAt: 1, windows: [{ label: "5h", percent: 78, resetsAt: null }, { label: "7d", percent: 41, resetsAt: null }] },
      usage: { input: 0, cachedInput: 0, cacheWrite: 0, output: 0, cost: { micros: 4_120_000, unit: "usd", estimated: true, models: [] } },
    };
    const line = "claude-opus-5 · eye · feat/x · PR #12 · ctx 42% · 5h 78% · 7d 41% · $4.12 est.";
    expect(statusLine(card)).toBe(line);
    const passed: Card = { ...card, limits: { ...card.limits!, windows: [{ label: "5h", percent: 78, resetsAt: 1 }, { label: "7d", percent: 41, resetsAt: null }] } };
    expect(statusLine(passed, 2_000), "a window whose reset passed").toBe("claude-opus-5 · eye · feat/x · PR #12 · ctx 42% · 5h reset · 7d 41% · $4.12 est.");
    const el = renderModal({ card, turns, status: null, draft: "", theme: "terminal" }, handlers(), 2_000);
    const foot = el.querySelector(".modal__composer + .modal__statusline--foot")!;
    expect(foot.textContent).toBe(line);
    expect(foot.classList.contains("modal__statusline--recorded")).toBe(false);
    expect(el.querySelector(".modal__titles .modal__statusline--head")?.textContent).toBe("claude-opus-5 · feat/x");
  });

  it("shows the foot in the terminal look and the header part in the chat look", () => {
    expect(styles).toContain(".modal__statusline--foot { display: none; }");
    expect(styles).toContain(".modal--terminal .modal__statusline--foot { display: block;");
    expect(styles).toContain(".modal--terminal .modal__statusline--head { display: none; }");
  });

  it("scrolls a recorded line wider than the panel inside the foot instead of overflowing it", () => {
    expect(styles).toContain(".modal__statusline--recorded { white-space: pre; overflow-x: auto; overflow-y: hidden; }");
    // No look sets the foot's overflow back.
    expect(styles).not.toMatch(/modal__statusline--(foot|recorded)[^{]*\{[^}]*overflow(-x)?: visible/);
  });

  it("a repaint carries the status lines and the working line along", () => {
    const root = renderModal({ card: base, turns, status: null, draft: "" }, handlers());
    patchModal(root, renderModal({ card: busy, turns, status: null, draft: "" }, handlers(), 2_000));
    expect(root.querySelector(".modal__statusline--head")?.textContent).toBe("claude-opus-5 · feat/x");
    expect(root.querySelector(".modal__statusline--foot")?.textContent).toBe("claude-opus-5 · eye · feat/x · PR #12 · ctx 42%");
    expect(root.querySelector(".modal__working")?.textContent).toBe("✻ Working… (1s)");
  });
});

describe("the open session view while a session asks", () => {
  const flush = () => new Promise((r) => setTimeout(r, 0));
  const host = () => document.getElementById("modal-host")!;
  const key = (target: EventTarget, k: string) => target.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }));
  const calls = (cmd: string) => invoke.mock.calls.filter(([c]) => c === cmd);

  beforeEach(() => {
    document.body.innerHTML = '<div id="modal-host"></div>';
    invoke.mockReset();
    invoke.mockImplementation((cmd: string) => Promise.resolve(cmd === "session_history" ? [] : null));
  });

  it("opening a window nudges its agent's account limits, and a failed nudge is ignored", async () => {
    invoke.mockImplementation((cmd: string) => (cmd === "nudge_limits" ? Promise.reject(new Error("gone")) : Promise.resolve(cmd === "session_history" ? [] : null)));
    await openModal({ ...base, sessionId: "m0", harness: "antigravity" });
    expect(calls("nudge_limits")).toEqual([["nudge_limits", { sessionId: "m0" }]]);
    expect(host().querySelector(".modal__status")?.textContent ?? "").toBe("");
    closeModal();
  });

  it("menu replaces the composer while asking and the composer returns after, with the draft", async () => {
    const working = { ...base, sessionId: "m1", state: "working" as const, stateSince: Date.now() - 5000 };
    await openModal(working);
    host().querySelector<HTMLTextAreaElement>(".modal__input")!.value = "half a thought";
    refreshModal([{ ...asking, sessionId: "m1", stateSince: Date.now() - 5000 }]);
    await flush();
    expect(host().querySelector(".menu")).not.toBeNull();
    expect(host().querySelector(".modal__input")).toBeNull();
    refreshModal([{ ...working, stateSince: Date.now() - 2000 }]);
    await flush();
    expect(host().querySelector(".menu")).toBeNull();
    expect(host().querySelector<HTMLTextAreaElement>(".modal__input")?.value).toBe("half a thought");
    closeModal();
  });

  it("keeps the menu, its focus and its own box through a repaint of the same question", async () => {
    const card = { ...asking, sessionId: "m2", stateSince: Date.now() - 5000 };
    await openModal(card);
    const box = host().querySelector<HTMLTextAreaElement>(".menu textarea")!;
    box.value = "Redis";
    box.focus();
    refreshModal([{ ...card, hasInbox: false }]);
    await flush();
    expect(host().querySelector(".menu textarea")).toBe(box);
    expect(document.activeElement).toBe(box);
    closeModal();
  });

  it("moves the usage strip on a board refresh that repaints nothing else, and leaves an unchanged one be", async () => {
    const card: Card = { ...base, sessionId: "u1", machine: "laptop" };
    await openModal(card);
    expect(host().querySelector(".usage__note")?.textContent).toBe("limits not read yet");
    const limits = { account: null, windows: [{ label: "5h", percent: 33, resetsAt: null }], readAt: Date.now() };
    refreshModal([{ ...card, limits }]);
    const strip = host().querySelector(".usage");
    expect(strip?.querySelector(".usage__meter[data-kind=limit] .usage__pct")?.textContent).toBe("33%");
    refreshModal([{ ...card, limits }]);
    expect(host().querySelector(".usage"), "the same strip, so a tooltip being read stays").toBe(strip);
    closeModal();
  });

  it("moves the recorded status line on a board refresh that repaints nothing else, and leaves an unchanged one be", async () => {
    const card: Card = { ...base, sessionId: "u2", machine: "laptop", statusLine: "Opus · 10%" };
    await openModal(card);
    const foot = () => host().querySelector(".modal__statusline--foot");
    expect(foot()?.textContent).toBe("Opus · 10%");
    refreshModal([{ ...card, statusLine: "\u001b[1mOpus\u001b[0m · 12%" }]);
    const moved = foot();
    expect(moved?.textContent).toBe("Opus · 12%");
    expect(moved?.querySelector(".ansi-b")?.textContent).toBe("Opus");
    refreshModal([{ ...card, statusLine: "\u001b[1mOpus\u001b[0m · 12%" }]);
    expect(foot(), "the same foot when it reads the same").toBe(moved);
    // Without a recording, Maya's own line follows the card's limits.
    const limits = { account: null, windows: [{ label: "5h", percent: 33, resetsAt: null }], readAt: Date.now() };
    refreshModal([{ ...card, statusLine: null, limits }]);
    expect(foot()?.textContent).toBe("eye · 5h 33%");
    expect(foot()?.classList.contains("modal__statusline--recorded")).toBe(false);
    closeModal();
  });

  it("Escape still closes the view while the menu is shown", async () => {
    await openModal({ ...asking, sessionId: "m3", stateSince: Date.now() - 5000 });
    const menu = host().querySelector<HTMLElement>(".menu")!;
    menu.focus();
    key(menu, "Escape");
    expect(host().childElementCount).toBe(0);
    await openModal({ ...asking, sessionId: "m3", stateSince: Date.now() - 5000 });
    key(host().querySelector(".menu textarea")!, "Escape");
    expect(host().childElementCount).toBe(0);
  });

  it("number keys act on the menu only once it has focus, and send the pick", async () => {
    const card = { ...asking, sessionId: "m4", stateSince: Date.now() - 5000 };
    await openModal(card);
    key(document, "1");
    expect(calls("answer_question")).toHaveLength(0);
    const menu = host().querySelector<HTMLElement>(".menu")!;
    menu.focus();
    key(menu, "1");
    expect(calls("answer_question")).toEqual([["answer_question", { sessionId: "m4", askId: card.stateSince, questionIndex: 0, optionIndex: 0 }]]);
    await flush();
    expect(host().querySelector(".modal__status")?.textContent).toBe("Answer sent to the terminal");
    closeModal();
  });

  it("Enter in the own box sends the words as the answer", async () => {
    const card = { ...asking, sessionId: "m5", stateSince: Date.now() - 5000 };
    await openModal(card);
    const box = host().querySelector<HTMLTextAreaElement>(".menu textarea")!;
    box.value = "Use Redis";
    key(box, "Enter");
    expect(calls("answer_pick")).toEqual([["answer_pick", { sessionId: "m5", askId: card.stateSince, questionIndex: 0, pick: { kind: "own", text: "Use Redis" } }]]);
    closeModal();
  });

  it("a second Stop is ignored until the card changes", async () => {
    const working = { ...base, sessionId: "m6", state: "working" as const, stateSince: Date.now() - 5000 };
    await openModal(working);
    host().querySelector<HTMLButtonElement>("button[data-action=stop]")!.click();
    expect(calls("stop_session")).toEqual([["stop_session", { sessionId: "m6" }]]);
    await flush();
    refreshModal([{ ...working }]);
    await flush();
    const again = host().querySelector<HTMLButtonElement>("button[data-action=stop]")!;
    expect(again.disabled).toBe(true);
    again.click();
    expect(calls("stop_session")).toHaveLength(1);
    refreshModal([{ ...working, stateSince: Date.now() - 2000 }]);
    await flush();
    expect(host().querySelector<HTMLButtonElement>("button[data-action=stop]")!.disabled).toBe(false);
    closeModal();
  });

  it("shows the backend's error in the status line when a stop or a pick fails", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "session_history") return Promise.resolve([]);
      return Promise.reject(cmd === "stop_session" ? "Maya needs Accessibility permission to press Esc in Terminal." : "The question has changed; pick again.");
    });
    const card = { ...asking, sessionId: "m7", stateSince: Date.now() - 5000 };
    await openModal(card);
    host().querySelector<HTMLButtonElement>("button[data-action=stop]")!.click();
    await flush();
    expect(host().querySelector(".modal__status")?.textContent).toBe("Maya needs Accessibility permission to press Esc in Terminal.");
    expect(host().querySelector<HTMLButtonElement>("button[data-action=stop]")!.disabled).toBe(false);
    host().querySelector<HTMLElement>(".menu [data-row='0']")!.click();
    await flush();
    expect(host().querySelector(".modal__status")?.textContent).toBe("The question has changed; pick again.");
    closeModal();
  });
});

describe("a Native session's live terminal", () => {
  const flush = () => new Promise((r) => setTimeout(r, 0));
  const host = () => document.getElementById("modal-host")!;
  const native: Card = { ...base, sessionId: "n1", pid: 42, native: true };
  const key = (target: EventTarget, k: string) => target.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }));

  beforeEach(() => {
    document.body.innerHTML = '<div id="modal-host"></div>';
    invoke.mockReset();
    invoke.mockImplementation((cmd: string) => Promise.resolve(cmd === "session_history" ? turns : null));
    live.mounts.length = 0;
  });

  it("is live only for a local Native card in the terminal look", () => {
    expect(isLive(native, "terminal")).toBe(true);
    expect(isLive(native, "chat")).toBe(false);
    expect(isLive(native, undefined)).toBe(false);
    expect(isLive({ ...native, machine: "laptop" }, "terminal")).toBe(false);
    expect(isLive(base, "terminal")).toBe(false);
  });

  it("a local Native card in the terminal look is a live terminal and nothing else", () => {
    for (const card of [native, { ...asking, native: true, stateSince: 0 }]) {
      const el = renderModal({ card, turns, status: null, draft: "", theme: "terminal" }, handlers(), 10_000);
      expect(el.querySelector(".modal__head")).not.toBeNull();
      expect(el.querySelector(".modal__live")).not.toBeNull();
      expect(el.querySelector(".modal__statusline--foot")).not.toBeNull();
      for (const gone of [".modal__composer", ".modal__banner", ".modal__history", ".menu", ".modal__tweaks", ".modal__chips"]) expect(el.querySelector(gone), gone).toBeNull();
      const panel = el.querySelector(".modal__panel")!;
      expect([...panel.children].map((c) => c.className)).toEqual(["modal__head", "modal__live", "modal__statusline modal__statusline--foot"]);
    }
  });

  it("the live terminal fills the panel between the head and the status line, in the look's colours", () => {
    expect(styles).toContain(".modal__live { flex: 1; min-height: 0; background: var(--term-bg); overflow: hidden; }");
    // Padded inside the terminal, which the fit add-on takes off, never on the parent it measures.
    expect(styles).toContain(".modal__live .xterm { height: 100%; padding: 8px 16px 4px; }");
    expect(styles).not.toMatch(/\.modal__live \{[^}]*padding/);
    expect(styles).toContain(".modal--terminal .modal__live { --term-bg: #0a0a0a; --term-fg: #e8e2d4;");
    expect(styles).toContain(':root[data-theme="light"] .modal--terminal .modal__live { --term-bg: #fffdf8; --term-fg: #231f14;');
  });

  it("a local Native card in the chat look keeps its reply box", () => {
    const el = renderModal({ card: native, turns, status: null, draft: "", theme: "chat" }, handlers());
    expect(el.querySelector(".modal__live")).toBeNull();
    expect(el.querySelector(".modal__composer")).not.toBeNull();
    expect(el.querySelector(".modal__history")).not.toBeNull();
  });

  it("a local Native card in the chat look is answered below, with no Open terminal button", () => {
    const ask = { ...asking, native: true, stateSince: 0 };
    const el = renderModal({ card: ask, turns, status: null, draft: "", theme: "chat" }, handlers(), 10_000);
    expect(el.querySelector(".modal__banner")?.textContent).toBe("This session is asking. Answer below.");
    expect(el.querySelector(".modal__banner button")).toBeNull();
    const prose = { ...base, native: true, state: "awaiting" as const, awaiting: { kind: "text" as const, detail: "Ship it?", questions: [] } };
    expect(renderModal({ card: prose, turns, status: null, draft: "" }, handlers()).querySelector(".modal__banner")?.textContent).toBe('This session asked you something. Reply below. "Ship it?"');
  });

  it("a remote Native card in the terminal look shows the transcript", () => {
    const el = renderModal({ card: { ...native, machine: "laptop" }, turns, status: null, draft: "", theme: "terminal" }, handlers());
    expect(el.querySelector(".modal__live")).toBeNull();
    expect(el.querySelector(".modal__history")).not.toBeNull();
  });

  it("a repaint keeps the live terminal's element", () => {
    const root = renderModal({ card: native, turns, status: null, draft: "", theme: "terminal" }, handlers());
    const node = root.querySelector(".modal__live");
    expect(node).not.toBeNull();
    patchModal(root, renderModal({ card: { ...native, state: "working" }, turns, status: null, draft: "", theme: "terminal" }, handlers()));
    expect(root.querySelector(".modal__live")).toBe(node);
    expect(root.querySelector(".modal__state")?.textContent).toBe("Working");
  });

  it("opening mounts the terminal once for the card's pid and focuses it; board refreshes keep it", async () => {
    setViewerTheme("terminal");
    await openModal(native);
    await flush();
    expect(live.mounts).toHaveLength(1);
    const [m] = live.mounts;
    expect(m.pid).toBe(42);
    expect(m.host).toBe(host().querySelector(".modal__live"));
    expect(m.focus).toHaveBeenCalled();
    refreshModal([{ ...native, state: "working", stateSince: 5 }]);
    await flush();
    refreshModal([{ ...native, state: "idle", stateSince: 9 }]);
    await flush();
    expect(live.mounts).toHaveLength(1);
    expect(host().querySelector(".modal__live")).toBe(m.host);
    expect(m.dispose).not.toHaveBeenCalled();
    closeModal();
    expect(m.dispose).toHaveBeenCalledTimes(1);
    setViewerTheme("chat");
  });

  it("switching to the chat look disposes the terminal, and back mounts a new one", async () => {
    setViewerTheme("terminal");
    await openModal(native);
    await flush();
    const [first] = live.mounts;
    setViewerTheme("chat");
    expect(first.dispose).toHaveBeenCalledTimes(1);
    expect(host().querySelector(".modal__live")).toBeNull();
    expect(host().querySelector(".modal__composer")).not.toBeNull();
    setViewerTheme("terminal");
    await flush();
    expect(live.mounts).toHaveLength(2);
    expect(host().querySelector(".modal__live")).toBe(live.mounts[1].host);
    closeModal();
    setViewerTheme("chat");
  });

  it("switching to the chat look before the terminal's code has loaded mounts nothing", async () => {
    setViewerTheme("terminal");
    const opening = openModal(native);
    setViewerTheme("chat");
    await opening;
    await flush();
    expect(live.mounts).toHaveLength(0);
    closeModal();
  });

  it("a Native card in the chat look mounts nothing", async () => {
    setViewerTheme("chat");
    await openModal(native);
    expect(live.mounts).toHaveLength(0);
    closeModal();
  });

  it("Escape inside the live terminal does not close the viewer", async () => {
    setViewerTheme("terminal");
    await openModal(native);
    const inside = document.createElement("textarea");
    host().querySelector(".modal__live")!.append(inside);
    key(inside, "Escape");
    expect(host().querySelector(".modal")).not.toBeNull();
    key(document.body, "Escape");
    expect(host().querySelector(".modal")).toBeNull();
    setViewerTheme("chat");
  });
});

describe("the viewer of a Native session with no card yet", () => {
  const flush = () => new Promise((r) => setTimeout(r, 0));
  const host = () => document.getElementById("modal-host")!;
  beforeEach(() => {
    closeModal();
    document.body.innerHTML = '<div id="modal-host"></div>';
    live.mounts.length = 0;
  });

  it("shows the folder and the live terminal, without a card's controls", () => {
    const onClose = vi.fn();
    const root = renderTermViewer("/x/dev/eye", onClose);
    expect(root.querySelector(".modal__title")?.textContent).toBe("eye");
    expect(root.classList.contains("modal--terminal")).toBe(true);
    expect(root.querySelector(".modal__live")).not.toBeNull();
    for (const sel of [".modal__theme", ".modal__state", ".modal__composer", ".modal__statusline", ".modal__harness"]) expect(root.querySelector(sel)).toBeNull();
    root.querySelector<HTMLButtonElement>(".modal__close")!.click();
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("closing the viewer before the terminal's code has loaded mounts nothing", async () => {
    openTermViewer(9, "/x/dev/eye");
    closeModal();
    await flush();
    expect(live.mounts).toHaveLength(0);
  });

  it("mounts the terminal by its id, and closing or opening a card disposes it", async () => {
    openTermViewer(9, "C:\\work\\eye");
    await flush();
    expect(host().querySelector(".modal__title")?.textContent).toBe("eye");
    expect(live.mounts.map((m) => [m.term, m.pid])).toEqual([[9, undefined]]);
    expect(live.mounts[0].host).toBe(host().querySelector(".modal__live"));
    expect(live.mounts[0].focus).toHaveBeenCalled();
    // Escape inside the terminal is the program's.
    live.mounts[0].host.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(live.mounts[0].dispose).not.toHaveBeenCalled();
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(live.mounts[0].dispose).toHaveBeenCalledTimes(1);
    expect(host().childElementCount).toBe(0);

    openTermViewer(9, "/x/dev/eye");
    await flush();
    invoke.mockResolvedValue([]);
    await openModal(base);
    expect(live.mounts[1].dispose).toHaveBeenCalledTimes(1);
    expect(host().querySelector(".modal__title")?.textContent).toBe("eye-1");
    closeModal();
  });
});
