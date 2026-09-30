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
    expect(on({ state: "awaiting-confirm" }).className).toContain("voice--awaiting");
    expect(on({ state: "thinking" }).className).toContain("voice--thinking");
    const err = on({ state: "error", detail: "no microphone" });
    expect(err.className).toContain("voice--error");
    expect(err.title).toContain("no microphone");
    expect(on({ level: 0.5 }).style.getPropertyValue("--level")).toBe("0.5");
  });
});

describe("voice panel", () => {
  it("shows the exchange and the pending read-back with yes and no, without a listen toggle", () => {
    const h = { onConfirm: vi.fn() };
    const el = renderVoicePanel(status({ heard: "tell hexgrid to go ahead", said: "Telling hexgrid: go ahead. Yes?", pending: "Telling hexgrid: go ahead. Yes?" }), [{ who: "user", text: "Maya what's waiting", at: 1 }, { who: "maya", text: "Nothing is waiting.", at: 2 }], h);
    expect([...el.querySelectorAll(".voice__turn")].map((t) => t.textContent)).toEqual(["Maya what's waiting", "Nothing is waiting."]);
    expect(el.querySelector(".voice__pending")?.textContent).toContain("go ahead");
    el.querySelector<HTMLButtonElement>("button[data-action=voice-yes]")!.click();
    expect(h.onConfirm).toHaveBeenCalledWith(true);
    el.querySelector<HTMLButtonElement>("button[data-action=voice-no]")!.click();
    expect(h.onConfirm).toHaveBeenCalledWith(false);
    expect(el.querySelector("input[name=listen]")).toBeNull();
    const quiet = renderVoicePanel(status(), [], h);
    expect(quiet.querySelector(".voice__pending")).toBeNull();
    expect(quiet.querySelector(".voice__empty")?.textContent).toContain("Say \"Maya\"");
  });
});
