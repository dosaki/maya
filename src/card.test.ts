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

describe("renderCard: PR button", () => {
  it("names a GitLab merge request MR !n and a GitHub one PR #n", () => {
    const gl = renderCard({ ...base, pr: { number: 12, url: "https://gitlab.com/g/r/-/merge_requests/12", state: "open", forge: "gitlab" } }, 0);
    expect(gl.querySelector("button[data-action=pr]")?.textContent).toBe("MR !12");
    const gh = renderCard({ ...base, pr: { number: 7, url: "https://github.com/o/r/pull/7", state: "open" } }, 0);
    expect(gh.querySelector("button[data-action=pr]")?.textContent).toBe("PR #7");
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

describe("renderCard: the Close button", () => {
  const original = navigator.userAgent;
  afterEach(() => Object.defineProperty(navigator, "userAgent", { value: original, configurable: true }));
  const close = (card: Card) => renderCard(card, 0).querySelector("button[data-action=close]");

  it("shows only on idle and completed cards of an agent that takes /exit", () => {
    expect(close(base)).not.toBeNull();
    expect(close({ ...base, state: "completed" })).not.toBeNull();
    expect(close({ ...base, state: "working" })).toBeNull();
    expect(close({ ...base, state: "awaiting" })).toBeNull();
    expect(close({ ...base, harness: "codex" })).not.toBeNull();
    expect(close({ ...base, harness: "kiro", hasInbox: false })).not.toBeNull();
    expect(close({ ...base, harness: "other", hasInbox: false })).toBeNull();
  });

  it("sits in the header's top-right corner, after the age", () => {
    const head = renderCard(base, 0).querySelector(".card__head")!;
    expect(head.lastElementChild?.matches("button.card__close[data-action=close]")).toBe(true);
    expect(head.lastElementChild?.previousElementSibling?.classList.contains("card__age")).toBe(true);
    expect(renderCard(base, 0).querySelector(".card__actions [data-action=close]")).toBeNull();
  });

  it("shows on Linux only when the session is in tmux, here or on the assistant", () => {
    Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (X11; Linux x86_64)", configurable: true });
    expect(close(base)).toBeNull();
    expect(close({ ...base, terminal: "maya-1a2b3c4d" })).not.toBeNull();
    Object.defineProperty(navigator, "userAgent", { value: original, configurable: true });
    expect(close({ ...base, machine: "box", machinePlatform: "linux" })).toBeNull();
    expect(close({ ...base, machine: "box", machinePlatform: "linux", terminal: "maya-1a2b3c4d" })).not.toBeNull();
    expect(close({ ...base, machine: "laptop", machinePlatform: "macos" })).not.toBeNull();
    expect(close({ ...base, machine: "laptop", machinePlatform: "macos", stale: true })).toBeNull();
  });
});

describe("renderCard: the Compact button", () => {
  const compact = (card: Card) => renderCard(card, 0).querySelector("button[data-action=compact]");
  const full = { used: 160_000, window: 200_000, percent: 80 };

  it("shows from 75 percent for every agent that has /compact", () => {
    expect(compact({ ...base, context: full })).not.toBeNull();
    expect(compact({ ...base, harness: "codex", hasInbox: false, context: full })).not.toBeNull();
    expect(compact({ ...base, harness: "antigravity", hasInbox: false, context: full })).not.toBeNull();
    expect(compact({ ...base, harness: "codex", hasInbox: false, context: { used: 140_000, window: 200_000, percent: 70 } })).toBeNull();
  });
});

describe("renderCard: options and Stop", () => {
  const question = { question: "Which?", header: "H", multiSelect: false, options: [{ label: "SQLite", description: "one file" }, { label: "Postgres", description: "" }], own: { label: "Type something", row: 2 }, chatRow: 3 };
  const asking: Card = { ...base, state: "awaiting", stateSince: 1000, awaiting: { kind: "question", detail: "Which?", questions: [question] } };

  it("card shows compact options and Stop", () => {
    const el = renderCard(asking, 5000);
    expect([...el.querySelectorAll(".options button[data-action=answer]")].map((b) => b.textContent)).toEqual(["SQLite", "Postgres"]);
    // Type something and Chat about this live in the session view.
    expect(el.textContent).not.toContain("Type something");
    expect(el.textContent).not.toContain("Chat about this");
    const stop = el.querySelector<HTMLButtonElement>(".card__actions button[data-action=stop]")!;
    expect(stop.textContent).toBe("Stop");
    expect(stop.disabled).toBe(false);
    expect(renderCard({ ...base, state: "working", stateSince: 1000 }, 5000).querySelector("button[data-action=stop]")).not.toBeNull();
    expect(renderCard({ ...base, state: "working", stateSince: 1000, shell: true }, 5000).querySelector("button[data-action=stop]")).toBeNull();
    expect(renderCard(base, 5000).querySelector("button[data-action=stop]")).toBeNull();
  });

  it("disables Stop and the options inside the open delay", () => {
    const el = renderCard(asking, 1500);
    expect(el.querySelector<HTMLButtonElement>("button[data-action=stop]")!.disabled).toBe(true);
    expect([...el.querySelectorAll<HTMLButtonElement>("button[data-action=answer]")].every((b) => b.disabled)).toBe(true);
  });

  it("offers the options of a permission or plan ask that has them", () => {
    const el = renderCard({ ...asking, awaiting: { kind: "permission", detail: "Bash: ls", questions: [question] } }, 5000);
    expect(el.querySelectorAll("button[data-action=answer]")).toHaveLength(2);
  });

  it("keeps the multi-select note on a multi-select card", () => {
    const el = renderCard({ ...asking, awaiting: { kind: "question", detail: "Which?", questions: [{ ...question, multiSelect: true, own: undefined, chatRow: undefined }] } }, 5000);
    expect(el.querySelector(".options__note")?.textContent).toBe("Multi-select: answer in the terminal");
  });
});
