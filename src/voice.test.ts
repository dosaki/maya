import { describe, expect, it, vi } from "vitest";
import { renderIndicator, renderVoicePanel, type VoiceStatus } from "./voice";

const status = (over: Partial<VoiceStatus> = {}): VoiceStatus => ({ listening: true, state: "idle", detail: "", level: 0, heard: "", said: "", pending: null, turns: 0, ...over });

describe("voice indicator", () => {
  it("reflects each state in a class and a label", () => {
    // The microphone appears only while listening is on; the toggle lives in Settings.
    expect(renderIndicator(status({ listening: false, state: "off" }))).toBeNull();
    const on = (over: Partial<VoiceStatus>) => renderIndicator(status(over))!;
    expect(on({}).className).toContain("voice--idle");
    expect(on({ state: "awaiting-command" }).className).toContain("voice--awaiting");
    expect(on({ state: "window" }).className).toContain("voice--awaiting");
    expect(on({ state: "thinking" }).className).toContain("voice--thinking");
    const err = on({ state: "error", detail: "no microphone" });
    expect(err.className).toContain("voice--error");
    expect(err.title).toContain("no microphone");
    expect(on({ level: 0.5 }).style.getPropertyValue("--level")).toBe("0.5");
  });
  it("names the wake word while idle, and Maya when none came", () => {
    expect(renderIndicator(status())!.title).toBe('Listening for "Maya".');
    expect(renderIndicator(status({ word: "Jarvis" }))!.title).toBe('Listening for "Jarvis".');
    expect(renderIndicator(status({ word: "Jarvis", state: "thinking" }))!.title).toBe("Thinking…");
  });
});

describe("voice panel", () => {
  it("shows the exchange, without a listen toggle", () => {
    const h = { onCancel: vi.fn() };
    const el = renderVoicePanel(status({ heard: "what's waiting", said: "Nothing is waiting." }), [{ who: "user", text: "Maya what's waiting", at: 1 }, { who: "maya", text: "Nothing is waiting.", at: 2 }], h);
    expect([...el.querySelectorAll(".voice__turn")].map((t) => t.textContent)).toEqual(["Maya what's waiting", "Nothing is waiting."]);
    expect(el.querySelector("input[name=listen]")).toBeNull();
    const quiet = renderVoicePanel(status(), [], h);
    expect(quiet.querySelector(".voice__pending")).toBeNull();
    expect(quiet.querySelector(".voice__empty")?.textContent).toContain("Say \"Maya\"");
    expect(renderVoicePanel(status({ word: "Jarvis" }), [], h).querySelector(".voice__empty")?.textContent).toContain('Say "Jarvis"');
  });

  it("shows a pending action with a countdown and a Cancel button", () => {
    const onCancel = vi.fn();
    const el = renderVoicePanel(status({ state: "window", pending: "Telling api-fix: open the PR.", until: Date.now() + 5000 }), [], { onCancel });
    expect(el.querySelector(".voice__pending")!.textContent).toContain("Telling api-fix");
    el.querySelector<HTMLButtonElement>("button[data-action=window-cancel]")!.click();
    expect(onCancel).toHaveBeenCalled();
    expect(el.querySelector("button[data-action=voice-yes]")).toBeNull();
  });

  it("counts the seconds left down from until, and shows none before the clock starts", () => {
    const h = { onCancel: vi.fn() };
    const left = (until: number | null) => renderVoicePanel(status({ state: "window", pending: "Starting a session in maya: fix it.", until }), [], h).querySelector(".voice__countdown")?.textContent;
    expect(left(Date.now() + 4200)).toBe("Running in 5 s.");
    expect(left(Date.now() + 900)).toBe("Running in 1 s.");
    expect(left(Date.now() - 10)).toBe("Running now.");
    expect(left(null)).toBeUndefined();
  });
});
