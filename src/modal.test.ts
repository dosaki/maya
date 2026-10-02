import { describe, expect, it, vi } from "vitest";
import { PANEL_SIZE_KEY, anchorPanel, draggedPanelSize, historyRefetchDue, isSlashCommand, onGrip, parsePanelSize, renderModal } from "./modal";
import type { Card, Turn } from "./types";

const base: Card = { sessionId: "s", pid: 1, name: "eye-1", cwd: "/x/dev/eye", state: "idle", stateSince: 0, snippet: "", awaiting: null, hasInbox: true, harness: "claude-code", pr: null, context: null };
const turns: Turn[] = [{ kind: "user", text: "hi" }, { kind: "assistant", text: "hello" }, { kind: "tool", text: "Bash: ls" }, { kind: "peer", text: "from eye" }];
const handlers = () => ({ onSend: vi.fn(), onTerminal: vi.fn(), onClose: vi.fn(), onAnswer: vi.fn(), onSetOption: vi.fn(), onCycleMode: vi.fn(), onOpenPr: vi.fn(), onRename: vi.fn(), onCommand: vi.fn(), onCompact: vi.fn(), onOpenLink: vi.fn() });

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

  it("pins the panel where it is as the pointer reaches its grip, so only that corner moves", () => {
    const rect = (left: number, top: number, width: number, height: number) => ({ left, top, right: left + width, bottom: top + height, width, height, x: left, y: top, toJSON: () => "" }) as DOMRect;
    expect(onGrip(rect(100, 50, 800, 600), 895, 645)).toBe(true);
    expect(onGrip(rect(100, 50, 800, 600), 870, 645)).toBe(false);
    expect(onGrip(rect(100, 50, 800, 600), 895, 620)).toBe(false);
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, handlers());
    const panel = el.querySelector<HTMLElement>(".modal__panel")!;
    el.getBoundingClientRect = () => rect(0, 0, 1280, 820);
    panel.getBoundingClientRect = () => rect(240, 110, 800, 600);
    panel.dispatchEvent(new MouseEvent("pointermove", { clientX: 500, clientY: 300, bubbles: true }));
    expect(el.classList.contains("modal--anchored")).toBe(false);
    panel.dispatchEvent(new MouseEvent("pointermove", { clientX: 1035, clientY: 705, bubbles: true }));
    expect(el.classList.contains("modal--anchored")).toBe(true);
    expect([panel.style.left, panel.style.top]).toEqual(["240px", "110px"]);
    expect(panel.style.maxWidth).toBe("calc(100vw - 248px)");
    // Pinning twice keeps the first position.
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
    expect([...el.querySelectorAll(".turn__who")].map((w) => w.textContent)).toEqual(["You", "Claude", "Message"]);
    expect(el.querySelector("textarea")).not.toBeNull();
    expect(el.querySelector(".modal__banner")).toBeNull();
  });

  it("keeps Claude-only controls off a Codex session but still lets you type a reply", () => {
    const h = handlers();
    const codex = { ...base, harness: "codex" as const, hasInbox: false, context: { used: 160_000, window: 200_000, percent: 80 } };
    const el = renderModal({ card: codex, turns: [], status: null, draft: "" }, h);
    expect(el.querySelector(".modal__tweaks")).toBeNull();
    expect(el.querySelector("button[data-action=compact]")).toBeNull();
    expect(el.querySelector(".modal__context")?.textContent).toBe("ctx 80%");
    el.querySelector<HTMLElement>(".modal__title")!.click();
    expect(el.querySelector("input.modal__title-input")).toBeNull();
    const ta = el.querySelector<HTMLTextAreaElement>("textarea")!;
    expect(ta.placeholder).toContain("typed into its terminal");
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
    expect(low.querySelector(".modal__context")?.textContent).toBe("ctx 10%");
    expect(low.querySelector("button[data-action=compact]")).toBeNull();
    const high = renderModal({ card: { ...base, context: { used: 160_000, window: 200_000, percent: 80 } }, turns: [], status: null, draft: "" }, h);
    const btn = high.querySelector<HTMLButtonElement>(".modal__head button[data-action=compact]")!;
    btn.click();
    expect(h.onCompact).toHaveBeenCalledTimes(1);
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

  it("still offers the tweaks row when the session has no inbox", () => {
    const el = renderModal({ card: { ...base, hasInbox: false }, turns: [], status: null, draft: "" }, handlers());
    expect(el.querySelector(".modal__tweaks")).not.toBeNull();
    expect(el.querySelector("textarea")).toBeNull();
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

  it("sends trimmed text on button click and on Cmd+Enter, never blank", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    const ta = el.querySelector<HTMLTextAreaElement>("textarea")!;
    ta.value = "   ";
    el.querySelector<HTMLButtonElement>("button[data-action=send]")!.click();
    expect(h.onSend).not.toHaveBeenCalled();
    ta.value = "  reply please  ";
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true }));
    expect(h.onSend).toHaveBeenCalledWith("reply please");
  });

  it("types a /command into the terminal instead of sending it as a reply, for Claude Code only", () => {
    expect(isSlashCommand("  /review")).toBe(true);
    expect(isSlashCommand("review /x")).toBe(false);
    expect(isSlashCommand("")).toBe(false);
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "", attachments: [{ name: "a.png", path: "/a.png" }] }, h);
    expect(el.querySelector<HTMLTextAreaElement>("textarea")!.placeholder).toContain("/command");
    const ta = el.querySelector<HTMLTextAreaElement>("textarea")!;
    ta.value = " /review the diff ";
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true }));
    expect(h.onCommand).toHaveBeenCalledWith("/review the diff");
    expect(h.onSend).not.toHaveBeenCalled();
    // Codex types every line into its terminal already: a slash line is just a reply there.
    const codex = renderModal({ card: { ...base, harness: "codex", hasInbox: false }, turns: [], status: null, draft: "" }, h);
    const ta2 = codex.querySelector<HTMLTextAreaElement>("textarea")!;
    ta2.value = "/status";
    codex.querySelector<HTMLButtonElement>("button[data-action=send]")!.click();
    expect(h.onSend).toHaveBeenCalledWith("/status");
    expect(h.onCommand).toHaveBeenCalledTimes(1);
  });

  it("shows the awaiting banner with a terminal button, and a status line", () => {
    const h = handlers();
    const el = renderModal({ card: { ...base, state: "awaiting", awaiting: { kind: "permission", detail: "Bash: rm", questions: [] } }, turns: [], status: { ok: false, text: "boom" }, draft: "" }, h);
    expect(el.querySelector(".modal__banner")?.textContent).toContain("waiting for a decision");
    const asking = { ...base, state: "awaiting" as const, stateSince: 5000, awaiting: { kind: "question" as const, detail: "Q?", questions: [{ question: "Q?", header: "H", multiSelect: false, options: [{ label: "A", description: "desc a" }] }] } };
    const early = renderModal({ card: asking, turns: [], status: null, draft: "", next: 0 }, h, 5500);
    expect(early.querySelector<HTMLButtonElement>(".modal__banner button[data-action=answer]")!.disabled).toBe(true);
    const q = renderModal({ card: asking, turns: [], status: null, draft: "", next: 0 }, h, 6000);
    expect(q.querySelector(".modal__banner .options__desc")?.textContent).toBe("desc a");
    const btn = q.querySelector<HTMLButtonElement>(".modal__banner button[data-action=answer]")!;
    expect(btn.disabled).toBe(false);
    btn.click();
    expect(h.onAnswer).toHaveBeenCalledWith(0, 0, btn);
    el.querySelector<HTMLButtonElement>(".modal__banner button")!.click();
    expect(h.onTerminal).toHaveBeenCalled();
    expect(el.querySelector(".modal__status")?.textContent).toBe("boom");
    expect(el.querySelector(".modal__status")?.className).toContain("modal__status--error");
  });

  it("replaces the composer when the session has no inbox", () => {
    const el = renderModal({ card: { ...base, hasInbox: false }, turns: [], status: null, draft: "" }, handlers());
    expect(el.querySelector("textarea")).toBeNull();
    expect(el.querySelector(".modal__noinbox")?.textContent).toContain("no inbox");
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

    const asking = { ...base, machine: "laptop", state: "awaiting" as const, stateSince: 5000, awaiting: { kind: "question" as const, detail: "Q?", questions: [{ question: "Q?", header: "H", multiSelect: false, options: [{ label: "A", description: "" }] }] } };
    const question = renderModal({ card: asking, turns: [], status: null, draft: "", next: 0 }, h, 6000);
    expect(question.querySelector(".modal__banner")?.textContent).not.toContain("terminal");
    expect(question.querySelector(".modal__banner")?.textContent).toContain("Pick an answer here.");
  });

  it("closes on backdrop click and on the close button", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    el.querySelector<HTMLElement>(".modal__backdrop")!.click();
    el.querySelector<HTMLButtonElement>("button[data-action=close]")!.click();
    expect(h.onClose).toHaveBeenCalledTimes(2);
  });
});
