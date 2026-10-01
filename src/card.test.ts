import { afterEach, describe, expect, it } from "vitest";
import { renderCard } from "./card";
import { remoteTerminalToast, type Card } from "./types";

const base: Card = {
  sessionId: "s",
  pid: 1,
  name: "eye-1",
  cwd: "/Users/x/dev/proj",
  state: "idle",
  stateSince: 0,
  snippet: "",
  awaiting: null,
  hasInbox: true,
  harness: "claude-code",
  pr: null,
  context: null,
};

describe("remoteTerminalToast", () => {
  it("says where a remote session runs, and how to attach when it is in tmux", () => {
    expect(remoteTerminalToast({ ...base, machine: "laptop" })).toBe("That session runs on laptop");
    expect(remoteTerminalToast({ ...base, machine: "box", terminal: "maya-1a2b3c4d" })).toBe("That session runs on box; attach: tmux attach -t maya-1a2b3c4d");
    expect(remoteTerminalToast({ ...base, machine: "box", terminal: null })).toBe("That session runs on box");
  });
});

describe("renderCard: remote cards", () => {
  it("marks a remote card with the machine and drops the Terminal button", () => {
    const el = renderCard({ ...base, machine: "laptop", stale: false }, 0);
    expect(el.querySelector(".card__remote")?.getAttribute("title")).toBe("Runs on laptop");
    expect(el.querySelector(".card__project")?.textContent).toBe("proj on laptop");
    expect(el.querySelector("button[data-action=terminal]")).toBeNull();
    expect(el.querySelector("button[data-action=reply]")).not.toBeNull();
    expect(renderCard({ ...base, machine: "laptop", stale: true }, 0).classList.contains("card--stale")).toBe(true);
    expect(renderCard(base, 0).querySelector("button[data-action=terminal]")).not.toBeNull();
  });

  it("names the machine's address in the glyph's tooltip", () => {
    const el = renderCard({ ...base, machine: "Gnowee (192.168.55.70)", machineAddress: "192.168.55.70", stale: false }, 0);
    // A label that already carries the address does not repeat it.
    expect(el.querySelector(".card__remote")?.getAttribute("title")).toBe("Runs on Gnowee (192.168.55.70)");
    const plain = renderCard({ ...base, machine: "Gnowee", machineAddress: "192.168.55.70", stale: false }, 0);
    expect(plain.querySelector(".card__remote")?.getAttribute("title")).toBe("Runs on Gnowee, 192.168.55.70");
    // With the platform: after the label, before the address.
    const mac = renderCard({ ...base, machine: "Gnowee", machineAddress: "192.168.55.70", machinePlatform: "macos", stale: false }, 0);
    expect(mac.querySelector(".card__remote")?.getAttribute("title")).toBe("Runs on Gnowee (macOS), 192.168.55.70");
    const twin = renderCard({ ...base, machine: "Gnowee (192.168.55.70)", machineAddress: "192.168.55.70", machinePlatform: "linux", stale: false }, 0);
    expect(twin.querySelector(".card__remote")?.getAttribute("title")).toBe("Runs on Gnowee (192.168.55.70) (Linux)");
    const other = renderCard({ ...base, machine: "box", machineAddress: "10.0.0.2", machinePlatform: "freebsd", stale: false }, 0);
    expect(other.querySelector(".card__remote")?.getAttribute("title")).toBe("Runs on box (freebsd), 10.0.0.2");
  });

  it("names the tmux session in the remote tooltip", () => {
    const el = renderCard({ ...base, machine: "laptop", machineAddress: "10.0.0.9", machinePlatform: "macos", stale: false, terminal: "maya-1a2b3c4d" }, 0);
    expect(el.querySelector(".card__remote")?.getAttribute("title")).toBe("Runs on laptop (macOS), 10.0.0.9; attach: tmux attach -t maya-1a2b3c4d");
  });

  it("shows no remote glyph or machine subtitle for a local card", () => {
    const el = renderCard(base, 0);
    expect(el.querySelector(".card__remote")).toBeNull();
    expect(el.querySelector(".card__project")?.textContent).toBe("proj");
    expect(el.classList.contains("card--stale")).toBe(false);
  });
});

describe("renderCard: the Terminal button on Linux", () => {
  const original = navigator.userAgent;
  const linuxUa = "Mozilla/5.0 (X11; Linux aarch64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

  afterEach(() => Object.defineProperty(navigator, "userAgent", { value: original, configurable: true }));

  it("shows the Terminal button only for a local card that is inside tmux", () => {
    Object.defineProperty(navigator, "userAgent", { value: linuxUa, configurable: true });
    const withTmux = renderCard({ ...base, terminal: "maya-1a2b3c4d" }, 0);
    const button = withTmux.querySelector<HTMLButtonElement>("button[data-action=terminal]");
    expect(button).not.toBeNull();
    expect(button?.title).toBe("Open the terminal attached to maya-1a2b3c4d");

    const withoutTmux = renderCard(base, 0);
    expect(withoutTmux.querySelector("button[data-action=terminal]")).toBeNull();
  });

  it("still shows the Terminal button for a local card on macOS and Windows, with or without tmux", () => {
    Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15", configurable: true });
    expect(renderCard(base, 0).querySelector("button[data-action=terminal]")).not.toBeNull();
    expect(renderCard({ ...base, terminal: "maya-1a2b3c4d" }, 0).querySelector("button[data-action=terminal]")).not.toBeNull();

    Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Edg/130.0", configurable: true });
    expect(renderCard(base, 0).querySelector("button[data-action=terminal]")).not.toBeNull();
    expect(renderCard({ ...base, terminal: "maya-1a2b3c4d" }, 0).querySelector("button[data-action=terminal]")).not.toBeNull();
  });
});
