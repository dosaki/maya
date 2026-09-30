import { describe, expect, it } from "vitest";
import { renderCard } from "./card";
import type { Card } from "./types";

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
