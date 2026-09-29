import { describe, expect, it, vi } from "vitest";
import { renderIndicator, renderVoicePanel, type VoiceStatus } from "./voice";

const status = (over: Partial<VoiceStatus> = {}): VoiceStatus => ({ listening: true, state: "idle", detail: "", level: 0, heard: "", said: "", pending: null, turns: 0, ...over });

describe("voice indicator", () => {
  it("reflects each state in a class and a label", () => {
    expect(renderIndicator(status({ listening: false, state: "off" })).className).toContain("voice--off");
    expect(renderIndicator(status()).className).toContain("voice--idle");
    expect(renderIndicator(status({ state: "awaiting-command" })).className).toContain("voice--awaiting");
    expect(renderIndicator(status({ state: "awaiting-confirm" })).className).toContain("voice--awaiting");
    expect(renderIndicator(status({ state: "thinking" })).className).toContain("voice--thinking");
    const err = renderIndicator(status({ state: "error", detail: "no microphone" }));
    expect(err.className).toContain("voice--error");
    expect(err.title).toContain("no microphone");
    expect(renderIndicator(status({ level: 0.5 })).style.getPropertyValue("--level")).toBe("0.5");
  });
});

describe("voice panel", () => {
  it("shows the exchange, the pending read-back with yes and no, and a listen toggle", () => {
    const h = { onConfirm: vi.fn(), onListen: vi.fn() };
    const el = renderVoicePanel(status({ heard: "tell hexgrid to go ahead", said: "Telling hexgrid: go ahead. Yes?", pending: "Telling hexgrid: go ahead. Yes?" }), [{ who: "user", text: "Maya what's waiting", at: 1 }, { who: "maya", text: "Nothing is waiting.", at: 2 }], h);
    expect([...el.querySelectorAll(".voice__turn")].map((t) => t.textContent)).toEqual(["Maya what's waiting", "Nothing is waiting."]);
    expect(el.querySelector(".voice__pending")?.textContent).toContain("go ahead");
    el.querySelector<HTMLButtonElement>("button[data-action=voice-yes]")!.click();
    expect(h.onConfirm).toHaveBeenCalledWith(true);
    el.querySelector<HTMLButtonElement>("button[data-action=voice-no]")!.click();
    expect(h.onConfirm).toHaveBeenCalledWith(false);
    const toggle = el.querySelector<HTMLInputElement>("input[name=listen]")!;
    expect(toggle.checked).toBe(true);
    toggle.checked = false;
    toggle.dispatchEvent(new Event("change"));
    expect(h.onListen).toHaveBeenCalledWith(false);
    const quiet = renderVoicePanel(status(), [], h);
    expect(quiet.querySelector(".voice__pending")).toBeNull();
    expect(quiet.querySelector(".voice__empty")?.textContent).toContain("Say \"Maya\"");
  });
});
