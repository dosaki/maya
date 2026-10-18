import { describe, expect, it, vi } from "vitest";
import { MIC_OFF, renderVoiceBar, type VoiceView } from "./voice";

const base: VoiceView = { state: "idle", heard: "", said: "", pending: null, error: null, offer: null, note: null };
const handlers = () => ({ onTap: vi.fn(), onConfirm: vi.fn(), onOpenSettings: vi.fn() });

describe("renderVoiceBar", () => {
  it("uses its own class names, not the desktop's 28 px .voice button", () => {
    const el = renderVoiceBar({ ...base, state: "listening", pending: "Telling x: y. Yes?", error: MIC_OFF, note: "n" }, handlers());
    expect(el.className).toBe("voicebar voicebar--listening");
    const classes = [el, ...el.querySelectorAll("*")].flatMap((e) => [...e.classList]);
    expect(classes.filter((c) => c === "voice" || c.startsWith("voice-") || c.startsWith("voice_"))).toEqual([]);
    expect(el.querySelector(".voicebar__mic")).not.toBeNull();
  });

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

  it("shows Yes and No during the read-back itself", () => {
    const h = handlers();
    const el = renderVoiceBar({ ...base, state: "speaking", pending: "Telling x: y. Yes?" }, h);
    el.querySelector<HTMLButtonElement>("button[data-action=yes]")!.click();
    expect(h.onConfirm.mock.calls).toEqual([[true]]);
    const done = renderVoiceBar({ ...base, state: "speaking", pending: null }, h);
    expect(done.querySelector("button[data-action=yes]")).toBeNull();
    expect(done.querySelector("button[data-action=no]")).toBeNull();
  });

  it("notes an online recogniser while idle", () => {
    const note = "Speech goes through Google's service and may leave the phone.";
    expect(renderVoiceBar({ ...base, note }, handlers()).querySelector(".voicebar__note")?.textContent).toBe(note);
    expect(renderVoiceBar({ ...base, note, state: "thinking" }, handlers()).querySelector(".voicebar__note")).toBeNull();
  });

  it("shows an error under the mic", () => {
    expect(renderVoiceBar({ ...base, error: "Microphone access is off." }, handlers()).querySelector(".voicebar__error")?.textContent).toBe("Microphone access is off.");
  });

  it("offers the app's settings when the microphone is refused", () => {
    expect(MIC_OFF).toBe("Microphone access is off.");
    const h = handlers();
    const el = renderVoiceBar({ ...base, error: MIC_OFF }, h);
    const open = el.querySelector<HTMLButtonElement>("button[data-action=open-settings]")!;
    expect(open.textContent).toBe("Open settings");
    open.click();
    expect(h.onOpenSettings).toHaveBeenCalled();
    expect(renderVoiceBar({ ...base, error: "no network for speech recognition" }, h).querySelector("button[data-action=open-settings]")).toBeNull();
    expect(renderVoiceBar(base, h).querySelector("button[data-action=open-settings]")).toBeNull();
  });
});
