import { describe, expect, it, vi } from "vitest";
import css from "./styles.css?raw";
import { morph } from "./morph";
import { renderDevices, type DevicesHandlers, type DevicesModel, type DeviceRow } from "./devices";
import type { NetworkStatus } from "./settings";

const handlers = (): DevicesHandlers => ({
  onRemoveMember: vi.fn(),
  onJoinAsk: vi.fn(),
  onJoin: vi.fn(),
  onJoinCancel: vi.fn(),
  onRemoveAssistant: vi.fn(),
  onMakeMain: vi.fn(),
  onMirror: vi.fn(),
  onPromote: vi.fn(),
  onPromoteCancel: vi.fn(),
});
const model = (extra: Partial<DevicesModel> = {}): DevicesModel => ({ joining: null, joinHost: "", joinPort: 0, promote: null, promoting: false, ...extra });
const dev = (extra: Partial<DeviceRow>): DeviceRow => ({ id: "x", key: "kx", name: "x", hostname: "x", platform: "macos", address: "", lastSeen: null, isMain: false, isSelf: false, ...extra });
const me = dev({ id: "m", key: "km", name: "laptop", address: "192.168.1.9", isSelf: true });
const desk = dev({ id: "d", key: "kd", name: "desk", address: "192.168.1.2", isMain: true, lastSeen: 1000 });
const nas = dev({ id: "n", key: "kn", name: "nas", address: "192.168.1.5", lastSeen: 1000 });
const pixel = dev({ id: "p", key: "kp", name: "Pixel", platform: "android", address: "192.168.1.30" });
const status = (connected: boolean, devices: DeviceRow[]): NetworkStatus => ({
  role: "assistant",
  code: null,
  assistants: [],
  devices,
  assistant: { connected, mainName: connected ? "desk" : null, error: null },
});

describe("renderDevices", () => {
  it("shows this device first, with its badge, then the main with the Main state", () => {
    const el = renderDevices(status(true, [desk, me, nas]), model(), handlers(), 3_601_000);
    const rows = [...el.querySelectorAll(".settings__assistant")];
    expect(rows[0].querySelector(".settings__device-name")?.textContent).toContain("laptop");
    expect(rows[0].querySelector(".settings__device-name .settings__badge")?.textContent).toBe("This device");
    expect(rows[0].querySelector(".settings__device-meta")?.textContent).toBe("macOS, 192.168.1.9");
    expect(rows[1].textContent).toContain("desk");
    expect(rows[1].querySelector(".settings__state--main")?.textContent).toBe("Main");
    expect(rows[1].querySelector(".settings__badge")).toBeNull();
    expect(rows[2].querySelector(".settings__badge")).toBeNull();
    expect(rows[2].querySelector(".settings__state--off")?.textContent).toBe("Last seen 1h ago");
  });

  it("calls this device 'This phone' when the phone renders it", () => {
    const el = renderDevices(status(true, [desk, me]), model(), handlers(), 2000, "this phone");
    expect(el.querySelector(".settings__badge")?.textContent).toBe("This phone");
  });

  it("states a connected assistant as Connected and an unseen one as Never connected", () => {
    const net: NetworkStatus = {
      role: "main",
      code: null,
      devices: [{ ...me, isMain: true }],
      assistants: [
        { id: "g", name: "g", hostname: "g", platform: "macos", connected: true, lastSeen: null },
        { id: "h", name: "h", hostname: "h", platform: "linux", connected: false, lastSeen: null },
      ],
      assistant: { connected: false, mainName: null, error: null },
    };
    const el = renderDevices(net, model(), handlers(), 2000);
    expect(el.querySelector("[data-row=g] .settings__state--on")?.textContent).toBe("Connected");
    expect(el.querySelector("[data-row=h] .settings__state--off")?.textContent).toBe("Never connected");
    expect(el.querySelector("[data-row=self\\:km] .settings__state--main")?.textContent).toBe("Main");
  });

  it("marks each row by its platform", () => {
    const win = dev({ id: "w", key: "kw", name: "w", platform: "windows" });
    const lin = dev({ id: "l", key: "kl", name: "l", platform: "linux" });
    const odd = dev({ id: "o", key: "ko", name: "o", platform: "freebsd" });
    const el = renderDevices(status(true, [me, nas, win, lin, pixel, odd]), model(), handlers(), 2000);
    expect([...el.querySelectorAll(".settings__device-mark")].map((m) => m.textContent)).toEqual(["Mac", "Mac", "Windows", "Linux", "Phone", "freebsd"]);
  });

  it("a main that is not connected says when it was last seen in its meta; a connected one adds nothing", () => {
    const away = renderDevices(status(false, [me, desk]), model(), handlers(), 3_601_000);
    expect(away.querySelector("[data-row=kd] .settings__device-meta")?.textContent).toBe("macOS, 192.168.1.2 — Last seen 1h ago");
    expect(away.querySelector("[data-row=kd] .settings__state--main")?.textContent).toBe("Main");
    const never = renderDevices(status(false, [me, { ...desk, lastSeen: null }]), model(), handlers(), 2000);
    expect(never.querySelector("[data-row=kd] .settings__device-meta")?.textContent).toBe("macOS, 192.168.1.2 — Never connected");
    const up = renderDevices(status(true, [me, desk]), model(), handlers(), 3_601_000);
    expect(up.querySelector("[data-row=kd] .settings__device-meta")?.textContent).toBe("macOS, 192.168.1.2");
  });

  it("puts the state under the name when asked", () => {
    const el = renderDevices(status(true, [me, nas]), model(), handlers(), 3_601_000, "this phone", { stateUnderName: true });
    const nasRow = el.querySelector("[data-row=kn]")!;
    const meta = nasRow.querySelector(".settings__device-meta")!;
    expect(meta.firstChild?.textContent).toBe("macOS, 192.168.1.5");
    expect(meta.querySelector(".settings__state")?.textContent).toBe("Last seen 1h ago");
    const side = renderDevices(status(true, [me, nas]), model(), handlers(), 3_601_000);
    expect(side.querySelector("[data-row=kn] .settings__device-meta .settings__state")).toBeNull();
    expect(side.querySelector("[data-row=kn] .settings__state")).not.toBeNull();
  });

  it("adds the hostname to rows whose names clash, and only those", () => {
    const a = dev({ id: "a", key: "ka", name: "Desk", hostname: "desk-a.lan", address: "192.168.1.2" });
    const b = dev({ id: "b", key: "kb", name: "desk", hostname: "desk-b.lan", address: "192.168.1.3" });
    const el = renderDevices(status(true, [me, a, b, nas]), model(), handlers(), 2000);
    const text = [...el.querySelectorAll(".settings__assistant")].map((r) => r.textContent ?? "");
    expect(text[1]).toContain("desk-a.lan");
    expect(text[2]).toContain("desk-b.lan");
    expect(text[3]).not.toContain(".lan");
    expect(text[0]).not.toContain(".lan");
  });

  it("offers Remove on other members and disables it while the main is unreachable", () => {
    const h = handlers();
    const up = renderDevices(status(true, [me, desk, nas]), model(), h, 2000);
    expect([...up.querySelectorAll<HTMLButtonElement>("button[data-action=remove-member]")].map((b) => b.dataset.key)).toEqual(["kn"]);
    const rm = up.querySelector<HTMLButtonElement>("button[data-action=remove-member]")!;
    expect(rm.disabled).toBe(false);
    rm.click();
    expect(h.onRemoveMember).toHaveBeenCalledWith("kn");
    const down = renderDevices(status(false, [me, desk, nas]), model(), h, 2000);
    const off = down.querySelector<HTMLButtonElement>("button[data-action=remove-member]")!;
    expect(off.disabled).toBe(true);
    expect(off.title).toBe("Connect to the main Maya to remove a device.");
  });

  it("offers no Join on a phone, on this device, or on the connected main", () => {
    const el = renderDevices(status(true, [me, desk, nas, pixel]), model(), handlers(), 2000);
    expect([...el.querySelectorAll<HTMLButtonElement>("button[data-action=join]")].map((b) => b.dataset.key)).toEqual(["kn"]);
    const away = renderDevices(status(false, [me, desk]), model(), handlers(), 2000);
    expect([...away.querySelectorAll<HTMLButtonElement>("button[data-action=join]")].map((b) => b.dataset.key)).toEqual(["kd"]);
  });

  it("Join asks for host and port prefilled from the last address, then calls onJoin", () => {
    const h = handlers();
    const net = status(true, [me, desk, nas]);
    renderDevices(net, model(), h, 2000).querySelector<HTMLButtonElement>("button[data-action=join]")!.click();
    expect(h.onJoinAsk).toHaveBeenCalledWith("kn");
    const el = renderDevices(net, model({ joining: "kn" }), h, 2000);
    expect(el.querySelector<HTMLInputElement>("input[name=join-host]")!.value).toBe("192.168.1.5");
    expect(el.querySelector<HTMLInputElement>("input[name=join-port]")!.value).toBe("4127");
    el.querySelector<HTMLButtonElement>("button[data-action=join-confirm]")!.click();
    expect(h.onJoin).toHaveBeenCalledWith("kn", "192.168.1.5", 4127);
  });

  it("Join prefills the main's last port when this Maya knows it", () => {
    const h = handlers();
    const away = dev({ ...desk, port: 5001 });
    const el = renderDevices(status(false, [me, away]), model({ joining: "kd" }), h, 2000);
    expect(el.querySelector<HTMLInputElement>("input[name=join-port]")!.value).toBe("5001");
    el.querySelector<HTMLButtonElement>("button[data-action=join-confirm]")!.click();
    expect(h.onJoin).toHaveBeenCalledWith("kd", expect.any(String), 5001);
  });

  it("on a main it lists the paired devices with Remove, Make main and the phone switch", () => {
    const h = handlers();
    const net: NetworkStatus = {
      role: "main",
      code: null,
      devices: [{ ...me, isMain: true }],
      assistants: [
        { id: "g", name: "laptop2", hostname: "l", platform: "macos", address: "192.168.1.7", connected: true, lastSeen: null, canLead: true },
        { id: "p", name: "Pixel", hostname: "p", platform: "android", address: "192.168.1.30", connected: true, lastSeen: null, wantsMirror: true, mirror: true },
      ],
      assistant: { connected: false, mainName: null, error: null },
    };
    const el = renderDevices(net, model(), h, 2000);
    expect(el.querySelector(".settings__assistant .settings__badge")?.textContent).toBe("This device");
    expect([...el.querySelectorAll<HTMLButtonElement>("button[data-action=remove-assistant]")].map((b) => b.dataset.id)).toEqual(["g", "p"]);
    el.querySelector<HTMLButtonElement>("button[data-action=make-main]")!.click();
    expect(h.onMakeMain).toHaveBeenCalledWith("g");
    const box = el.querySelector<HTMLInputElement>("input[data-action=mirror]")!;
    expect(box.parentElement?.textContent).toContain("Sees and controls all sessions");
    expect(box.classList.contains("settings__toggle")).toBe(true);
    expect(box.closest(".settings__device-text")).not.toBeNull();
    expect(box.getAttribute("aria-label")).toBe("Sees and controls all sessions");
    box.checked = false;
    box.dispatchEvent(new Event("change", { bubbles: true }));
    expect(h.onMirror).toHaveBeenCalledWith("p", false);
    expect(el.querySelector("button[data-action=join]")).toBeNull();
  });
});

describe("the Devices stylesheet", () => {
  it("scopes every device rule to .settings.pane and keeps the row direction over the column rule", () => {
    const rules = css.split("\n").filter((l) => /^\S*\.settings__(device|devices|assistant|assistants|badge)\b/.test(l.trim()) || /^\.settings\.pane \.settings__(device|devices|assistant|assistants|badge)/.test(l));
    expect(rules.length).toBeGreaterThan(8);
    for (const l of css.split("\n")) if (/^\.settings__(device|devices|assistant|assistants|badge)/.test(l)) throw new Error("unscoped: " + l);
    expect(css).toMatch(/\.settings\.pane \.settings__assistant \{[^}]*flex-direction: column/);
    expect(css).toMatch(/\.settings\.pane \.settings__device \{[^}]*flex-direction: row/);
    expect(css.indexOf(".settings.pane .settings__device {")).toBeGreaterThan(css.indexOf(".settings.pane .settings__assistant {"));
  });
});

// The Settings repaint keeps the first render's list and patches it against
// each new render, so the kept root's listener must act on the new rows.
describe("after a repaint", () => {
  const patch = (old: HTMLElement, fresh: HTMLElement) => morph(old, fresh);
  const cell = (c: HTMLElement, sel: string) => c.querySelector<HTMLButtonElement>(sel)!;
  const main = (promote: string | null, promoting = false): NetworkStatus => ({
    role: "main",
    code: null,
    devices: [{ ...me, isMain: true }],
    assistants: [
      { id: "g", name: "g", hostname: "g", platform: "macos", connected: true, lastSeen: null, canLead: true },
      { id: "h", name: "h", hostname: "h", platform: "macos", connected: true, lastSeen: null, canLead: true },
    ],
    assistant: { connected: false, mainName: null, error: null },
    ...(promote || promoting ? {} : {}),
  });

  it("promote confirm acts on the device it was switched to", () => {
    const h = handlers();
    const kept = renderDevices(main("g"), model({ promote: "g" }), h, 0);
    patch(kept, renderDevices(main("h"), model({ promote: "h" }), h, 0));
    cell(kept, "button[data-action=promote-confirm]").click();
    expect(h.onPromote).toHaveBeenCalledWith("h");
    expect(h.onPromote).not.toHaveBeenCalledWith("g");
  });

  it("Join confirm uses the key and host of the row whose form is open now", () => {
    const h = handlers();
    const desk2 = dev({ id: "d", key: "kd", name: "desk", address: "192.168.1.2", lastSeen: 1 });
    const net = status(true, [me, desk2, nas]);
    const kept = renderDevices(net, model({ joining: "kn" }), h, 0);
    patch(kept, renderDevices(net, model({ joining: "kd" }), h, 0));
    cell(kept, "button[data-action=join-confirm]").click();
    expect(h.onJoin).toHaveBeenCalledWith("kd", "192.168.1.2", 4127);
  });

  it("a member that becomes the main gets Join, and Join never removes it", () => {
    const h = handlers();
    const before = status(true, [me, { ...desk, isMain: false }]);
    const kept = renderDevices(before, model(), h, 0);
    expect(kept.querySelector("[data-action=remove-member]")).not.toBeNull();
    patch(kept, renderDevices(status(false, [me, desk]), model(), h, 0));
    expect(kept.querySelector("[data-action=remove-member]")).toBeNull();
    cell(kept, "button[data-action=join]").click();
    expect(h.onJoinAsk).toHaveBeenCalledWith("kd");
    expect(h.onRemoveMember).not.toHaveBeenCalled();
  });
});

describe("renderDevices markup", () => {
  const order = (root: ParentNode, sel: string) => [...root.querySelectorAll<HTMLButtonElement>(`${sel} button`)].map((b) => b.dataset.action);

  it("puts Remove last in a member's actions, so tab order matches what is on screen", () => {
    const el = renderDevices(status(true, [me, desk, nas]), model(), handlers(), 2000);
    expect(order(el, "[data-row=kn] .settings__assistant-actions")).toEqual(["join", "remove-member"]);
  });

  it("puts Remove last in an assistant's actions too", () => {
    const net: NetworkStatus = {
      role: "main",
      code: null,
      assistants: [{ id: "a1", name: "Gnowee", platform: "linux", address: "", connected: true, lastSeen: 1, canLead: true } as never],
      devices: [dev({ ...me, isMain: true })],
      assistant: { connected: false, mainName: null, error: null },
    };
    const el = renderDevices(net, model(), handlers(), 2000);
    expect(order(el, "[data-row=a1] .settings__assistant-actions")).toEqual(["make-main", "remove-assistant"]);
  });

  it("names each platform: macOS, Windows, Linux, Android", () => {
    const rows = [dev({ id: "1", key: "k1", name: "a", platform: "macos" }), dev({ id: "2", key: "k2", name: "b", platform: "windows" }), dev({ id: "3", key: "k3", name: "c", platform: "linux" }), dev({ id: "4", key: "k4", name: "d", platform: "android" })];
    const el = renderDevices(status(true, [me, ...rows]), model(), handlers(), 2000);
    const metas = [...el.querySelectorAll(".settings__device-meta")].map((m) => m.firstChild?.textContent);
    expect(metas.slice(1)).toEqual(["macOS", "Windows", "Linux", "Android"]);
  });

  it("draws the Join form with a plain Join and a quiet Cancel, keeping its names and data", () => {
    const el = renderDevices(status(true, [me, desk, nas]), model({ joining: "kn" }), handlers(), 2000);
    const form = el.querySelector<HTMLElement>("[data-for=join-form]")!;
    expect(form.dataset.row).toBe("join:kn");
    expect([...form.children].map((c) => (c as HTMLElement).getAttribute("name") ?? (c as HTMLElement).dataset.action)).toEqual(["join-host", "join-port", "join-confirm", "join-cancel"]);
    expect(form.querySelector("[data-action=join-confirm]")!.className).toBe("settings__btn");
    expect(form.querySelector("[data-action=join-cancel]")!.className).toBe("settings__btn settings__btn--quiet");
    expect(form.querySelector("input[name=join-host]")!.getAttribute("aria-label")).toBe("Host or address");
    expect(form.querySelector("input[name=join-port]")!.getAttribute("aria-label")).toBe("Port");
  });
});
