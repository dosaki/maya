import { describe, expect, it, vi } from "vitest";
import { renderVoiceBar, type VoiceView } from "./voice";

const base: VoiceView = { state: "idle", heard: "", said: "", pending: null, error: null, offer: null, note: null };
const handlers = () => ({ onTap: vi.fn(), onConfirm: vi.fn() });

describe("renderVoiceBar", () => {
  it("offers the mic when voice is offered, and taps through", () => {
    const h = handlers();
    const el = renderVoiceBar(base, h);
    const mic = el.querySelector<HTMLButtonElement>("button[data-action=mic]")!;
    expect(mic.disabled).toBe(false);
    mic.click();
    expect(h.onTap).toHaveBeenCalled();
  });

  it("disables the mic with the reason when voice is not offered", () => {
    const el = renderVoiceBar({ ...base, state: "off", offer: "Update Yhi to talk to Maya." }, handlers());
    expect(el.querySelector<HTMLButtonElement>("button[data-action=mic]")!.disabled).toBe(true);
    expect(el.textContent).toContain("Update Yhi to talk to Maya.");
  });

  it("hides entirely when the phone is its own main", () => {
    const el = renderVoiceBar({ ...base, state: "off", offer: "Voice needs a desktop main." }, handlers());
    expect(el.hidden).toBe(true);
  });

  it("shows what was heard and said, and Thinking while the main works", () => {
    expect(renderVoiceBar({ ...base, heard: "what needs me", said: "Two sessions need you." }, handlers()).textContent).toContain("what needs me");
    expect(renderVoiceBar({ ...base, said: "Two sessions need you." }, handlers()).textContent).toContain("Two sessions need you.");
    expect(renderVoiceBar({ ...base, state: "thinking" }, handlers()).textContent).toContain("Thinking…");
  });

  it("shows Yes and No while a read-back waits", () => {
    const h = handlers();
    const el = renderVoiceBar({ ...base, state: "awaiting-confirm", pending: "Telling x: y. Yes?" }, h);
    el.querySelector<HTMLButtonElement>("button[data-action=yes]")!.click();
    el.querySelector<HTMLButtonElement>("button[data-action=no]")!.click();
    expect(h.onConfirm.mock.calls).toEqual([[true], [false]]);
    expect(renderVoiceBar(base, h).querySelector("button[data-action=yes]")).toBeNull();
  });

  it("notes an online recogniser while idle", () => {
    const note = "Speech goes through Google's service and may leave the phone.";
    expect(renderVoiceBar({ ...base, note }, handlers()).querySelector(".voice__note")?.textContent).toBe(note);
    expect(renderVoiceBar({ ...base, note, state: "thinking" }, handlers()).querySelector(".voice__note")).toBeNull();
  });

  it("shows an error under the mic", () => {
    expect(renderVoiceBar({ ...base, error: "Microphone access is off." }, handlers()).querySelector(".voice__error")?.textContent).toBe("Microphone access is off.");
  });
});
