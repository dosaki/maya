import { describe, expect, it, vi } from "vitest";
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
  it("shows this device first, then the main with a badge", () => {
    const el = renderDevices(status(true, [desk, me, nas]), model(), handlers(), 2000);
    const rows = [...el.querySelectorAll(".settings__assistant")];
    expect(rows[0].textContent).toContain("This device");
    expect(rows[0].textContent).toContain("laptop");
    expect(rows[1].textContent).toContain("desk");
    expect(rows[1].querySelector(".settings__badge")?.textContent).toBe("Main");
    expect(rows[2].querySelector(".settings__badge")).toBeNull();
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
    expect(el.querySelector(".settings__assistant")?.textContent).toContain("This device");
    expect([...el.querySelectorAll<HTMLButtonElement>("button[data-action=remove-assistant]")].map((b) => b.dataset.id)).toEqual(["g", "p"]);
    el.querySelector<HTMLButtonElement>("button[data-action=make-main]")!.click();
    expect(h.onMakeMain).toHaveBeenCalledWith("g");
    const box = el.querySelector<HTMLInputElement>("input[data-action=mirror]")!;
    expect(box.parentElement?.textContent).toContain("Sees and controls all sessions");
    box.checked = false;
    box.dispatchEvent(new Event("change"));
    expect(h.onMirror).toHaveBeenCalledWith("p", false);
    expect(el.querySelector("button[data-action=join]")).toBeNull();
  });
});
