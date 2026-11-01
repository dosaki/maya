import { beforeEach, describe, expect, it, vi } from "vitest";

const { FakeTerminal, FakeFit, FakeChannel } = vi.hoisted(() => {
  /** The fake xterm the tests drive: it records what was written and done to it. */
  class FakeTerminal {
    static last: FakeTerminal | null = null;
    options: Record<string, unknown>;
    rows = 24;
    cols = 80;
    calls: string[] = [];
    written: Uint8Array[] = [];
    text: string[] = [];
    dataListener: ((d: string) => void) | null = null;
    keyHandler: ((ev: KeyboardEvent) => boolean) | null = null;
    selection = "";
    pasted: string[] = [];
    disposed = false;
    modes = { applicationCursorKeysMode: false };
    constructor(options: Record<string, unknown>) {
      this.options = options;
      FakeTerminal.last = this;
    }
    loadAddon(addon: { activate(t: FakeTerminal): void }) {
      addon.activate(this);
    }
    open(host: HTMLElement) {
      this.calls.push(`open ${host.className}`);
    }
    resize(cols: number, rows: number) {
      this.calls.push(`resize ${rows}x${cols}`);
      this.rows = rows;
      this.cols = cols;
    }
    reset() {
      this.calls.push("reset");
    }
    /** Callbacks of writes not parsed yet: xterm parses writes later, but resizes at once. */
    unparsed: (() => void)[] = [];
    write(data: string | Uint8Array, parsed?: () => void) {
      if (parsed) this.unparsed.push(parsed);
      if (typeof data === "string") {
        this.calls.push(`write ${data}`);
        this.text.push(data);
      } else {
        const s = new TextDecoder().decode(data);
        this.calls.push(`write ${s}`);
        this.written.push(data);
        this.text.push(s);
      }
    }
    /** Finishes parsing what was written so far. */
    parse() {
      const done = this.unparsed;
      this.unparsed = [];
      for (const f of done) f();
    }
    onData(f: (d: string) => void) {
      this.dataListener = f;
      return { dispose() {} };
    }
    onBinary() {
      return { dispose() {} };
    }
    attachCustomKeyEventHandler(f: (ev: KeyboardEvent) => boolean) {
      this.keyHandler = f;
    }
    getSelection() {
      return this.selection;
    }
    paste(s: string) {
      this.pasted.push(s);
    }
    focus() {
      this.calls.push("focus");
    }
    dispose() {
      this.disposed = true;
    }
  }

  /** The fake fit add-on: fit() resizes its terminal to `proposed`. */
  class FakeFit {
    static proposed = { rows: 40, cols: 120 };
    term: FakeTerminal | null = null;
    activate(t: FakeTerminal) {
      this.term = t;
    }
    fit() {
      const { rows, cols } = FakeFit.proposed;
      if (this.term && (this.term.rows !== rows || this.term.cols !== cols)) this.term.resize(cols, rows);
    }
    dispose() {}
  }

  class FakeChannel<T> {
    static last: FakeChannel<unknown> | null = null;
    onmessage: (m: T) => void = () => {};
    constructor() {
      FakeChannel.last = this as FakeChannel<unknown>;
    }
  }

  return { FakeTerminal, FakeFit, FakeChannel };
});

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args), Channel: FakeChannel }));
vi.mock("@xterm/xterm", () => ({ Terminal: FakeTerminal }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: FakeFit }));
const platform = vi.hoisted(() => ({ mac: false }));
vi.mock("./platform", () => ({ isMac: () => platform.mac }));

let observed: (() => void) | null = null;
class FakeObserver {
  constructor(cb: () => void) {
    observed = cb;
  }
  observe() {}
  disconnect() {
    observed = null;
  }
}
vi.stubGlobal("ResizeObserver", FakeObserver);

import { EXTRA_KEYS, LIGHT_ANSI, REATTACH_MS, copyPasteKey, fitFont, keyBytes, mountLive } from "./live";

const flush = () => new Promise((r) => setTimeout(r, 0));
const attached = { term: 7, viewer: 1, snapshot: "aGk=", rows: 40, cols: 120 };
const term = () => FakeTerminal.last!;
const channel = () => FakeChannel.last as { onmessage: (m: { kind: string; data?: string }) => void };
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd);

function host(): HTMLElement {
  const h = document.createElement("div");
  h.className = "modal__live";
  document.body.append(h);
  return h;
}

async function mounted() {
  invoke.mockImplementation((cmd: string) => (cmd === "native_attach" ? Promise.resolve(attached) : Promise.resolve(null)));
  const handle = mountLive(host(), { pid: 42 });
  await flush();
  term().parse();
  return handle;
}

describe("mountLive", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    invoke.mockReset();
    FakeFit.proposed = { rows: 40, cols: 120 };
    FakeTerminal.last = null;
    observed = null;
    platform.mac = false;
    delete document.documentElement.dataset.theme;
  });

  it("opens a terminal with 10,000 lines of scrollback", async () => {
    await mounted();
    expect(term().options.scrollback).toBe(10000);
    expect(term().calls[0]).toBe("open modal__live");
  });

  it("attaches by pid, writes the snapshot, then streamed output", async () => {
    await mounted();
    expect(calls("native_attach")).toEqual([["native_attach", { pid: 42, onFrame: channel() }]]);
    expect(term().text).toEqual(["hi"]);
    channel().onmessage({ kind: "output", data: "eA==" });
    expect(term().text).toEqual(["hi", "x"]);
    expect(term().written.every((w) => w instanceof Uint8Array)).toBe(true);
  });

  it("attaches by terminal id to a session that has no card yet", async () => {
    invoke.mockImplementation((cmd: string) => (cmd === "native_attach_term" ? Promise.resolve(attached) : Promise.resolve(null)));
    mountLive(host(), { term: 9 });
    await flush();
    expect(calls("native_attach")).toEqual([]);
    expect(calls("native_attach_term")).toEqual([["native_attach_term", { term: 9, onFrame: channel() }]]);
    expect(term().text).toEqual(["hi"]);
  });

  it("resizes to the attached size before writing the snapshot, and fits only once it is parsed", async () => {
    FakeFit.proposed = { rows: 30, cols: 100 };
    invoke.mockImplementation((cmd: string) => (cmd === "native_attach" ? Promise.resolve(attached) : Promise.resolve(null)));
    mountLive(host(), { pid: 42 });
    // No fit before the snapshot is drawn: the panel's size would lay it out wrong.
    observed!();
    await flush();
    const c = term().calls;
    const sized = c.indexOf("resize 40x120");
    const snapshot = c.indexOf("write hi");
    expect(sized).toBeGreaterThan(-1);
    expect(sized).toBeLessThan(snapshot);
    // Written but not parsed yet: still at the attached size, and the program not told.
    expect(c).not.toContain("resize 30x100");
    observed!();
    expect(c).not.toContain("resize 30x100");
    expect(calls("native_resize")).toEqual([]);
    term().parse();
    await flush();
    expect(c.lastIndexOf("resize 30x100")).toBeGreaterThan(snapshot);
    expect(calls("native_resize")).toEqual([["native_resize", { term: 7, rows: 30, cols: 100 }]]);
  });

  it("queues frames that arrive before the attach resolves and writes them after the snapshot", async () => {
    let resolve: (a: typeof attached) => void = () => {};
    invoke.mockImplementation((cmd: string) => (cmd === "native_attach" ? new Promise((r) => (resolve = r)) : Promise.resolve(null)));
    mountLive(host(), { pid: 42 });
    channel().onmessage({ kind: "output", data: "YQ==" });
    channel().onmessage({ kind: "output", data: "Yg==" });
    expect(term().text).toEqual([]);
    resolve(attached);
    await flush();
    expect(term().text).toEqual(["hi", "a", "b"]);
  });

  it("sends typed keys as base64 to native_input", async () => {
    await mounted();
    term().dataListener!("é");
    expect(calls("native_input")).toEqual([["native_input", { term: 7, data: "w6k=" }]]);
  });

  it("resizes the terminal to the fitted size", async () => {
    await mounted();
    expect(calls("native_resize")).toEqual([]);
    FakeFit.proposed = { rows: 30, cols: 100 };
    observed!();
    await flush();
    expect(calls("native_resize")).toEqual([["native_resize", { term: 7, rows: 30, cols: 100 }]]);
    // The same size again is not sent twice.
    observed!();
    await flush();
    expect(calls("native_resize")).toHaveLength(1);
  });

  it("sends a resize only once the one before it is done, so the last size wins", async () => {
    await mounted();
    let done: () => void = () => {};
    invoke.mockImplementation((cmd: string) => (cmd === "native_resize" ? new Promise<void>((r) => (done = r)) : Promise.resolve(null)));
    FakeFit.proposed = { rows: 30, cols: 100 };
    observed!();
    FakeFit.proposed = { rows: 20, cols: 90 };
    observed!();
    await flush();
    expect(calls("native_resize")).toEqual([["native_resize", { term: 7, rows: 30, cols: 100 }]]);
    done();
    await flush();
    expect(calls("native_resize").map((c) => c[1])).toEqual([
      { term: 7, rows: 30, cols: 100 },
      { term: 7, rows: 20, cols: 90 },
    ]);
  });

  it("a local viewer ignores size frames", async () => {
    await mounted();
    channel().onmessage({ kind: "size", rows: 30, cols: 100 } as { kind: string });
    await flush();
    expect(term().calls).not.toContain("resize 30x100");
    expect(calls("native_resize")).toEqual([]);
    expect(term().text.join("")).not.toContain("[Session ended]");
    term().dataListener!("x");
    expect(calls("native_input")).toHaveLength(1);
  });

  it("shows that the session ended on an exit frame and stops sending input", async () => {
    await mounted();
    channel().onmessage({ kind: "exit" });
    expect(term().text.join("")).toContain("[Session ended]");
    term().dataListener!("x");
    expect(calls("native_input")).toEqual([]);
  });

  it("an exit queued before the attach resolves is shown after the snapshot", async () => {
    let resolve: (a: typeof attached) => void = () => {};
    invoke.mockImplementation((cmd: string) => (cmd === "native_attach" ? new Promise((r) => (resolve = r)) : Promise.resolve(null)));
    mountLive(host(), { pid: 42 });
    channel().onmessage({ kind: "exit" });
    resolve(attached);
    await flush();
    expect(term().text[0]).toBe("hi");
    expect(term().text.slice(1).join("")).toContain("[Session ended]");
  });

  it("shows why it could not attach", async () => {
    invoke.mockImplementation((cmd: string) => (cmd === "native_attach" ? Promise.reject("This session is not running in Maya.") : Promise.resolve(null)));
    mountLive(host(), { pid: 42 });
    await flush();
    expect(term().text.join("")).toContain("This session is not running in Maya.");
    term().dataListener!("x");
    expect(calls("native_input")).toEqual([]);
  });

  it("detaches on dispose", async () => {
    const handle = await mounted();
    handle.dispose();
    expect(calls("native_detach")).toEqual([["native_detach", { term: 7, viewer: 1 }]]);
    expect(term().disposed).toBe(true);
    expect(observed).toBeNull();
  });

  it("detaches at once when disposed before the attach resolves", async () => {
    let resolve: (a: typeof attached) => void = () => {};
    invoke.mockImplementation((cmd: string) => (cmd === "native_attach" ? new Promise((r) => (resolve = r)) : Promise.resolve(null)));
    const handle = mountLive(host(), { pid: 42 });
    handle.dispose();
    resolve(attached);
    await flush();
    expect(calls("native_detach")).toEqual([["native_detach", { term: 7, viewer: 1 }]]);
    expect(term().text).toEqual([]);
  });

  const clipboard = () => {
    const writeText = vi.fn(() => Promise.resolve());
    const readText = vi.fn(() => Promise.resolve("pasted"));
    Object.defineProperty(navigator, "clipboard", { value: { writeText, readText }, configurable: true });
    return { writeText, readText };
  };
  const press = (key: string, mods: Partial<KeyboardEventInit>) => {
    const ev = new KeyboardEvent("keydown", { key, cancelable: true, ...mods });
    return { kept: term().keyHandler!(ev), prevented: ev.defaultPrevented };
  };

  it("on macOS copies the selection on Cmd+C and leaves Cmd+V to the webview's own paste, which xterm takes", async () => {
    platform.mac = true;
    const { writeText, readText } = clipboard();
    await mounted();
    term().selection = "chosen";
    expect(press("c", { metaKey: true })).toEqual({ kept: false, prevented: true });
    expect(writeText).toHaveBeenCalledWith("chosen");
    // Not prevented: the native paste event reaches xterm's textarea, with no WebKit Paste prompt.
    expect(press("v", { metaKey: true })).toEqual({ kept: false, prevented: false });
    await flush();
    expect(readText).not.toHaveBeenCalled();
    expect(term().pasted).toEqual([]);
    expect(press("a", {}).kept).toBe(true);
  });

  it("elsewhere copies on Ctrl+Shift+C and pastes the clipboard on Ctrl+Shift+V; Ctrl+V is the program's", async () => {
    const { writeText, readText } = clipboard();
    await mounted();
    term().selection = "chosen";
    expect(press("C", { ctrlKey: true, shiftKey: true })).toEqual({ kept: false, prevented: true });
    expect(writeText).toHaveBeenCalledWith("chosen");
    expect(press("V", { ctrlKey: true, shiftKey: true })).toEqual({ kept: false, prevented: true });
    await flush();
    expect(readText).toHaveBeenCalled();
    expect(term().pasted).toEqual(["pasted"]);
    expect(press("v", { ctrlKey: true }).kept).toBe(true);
  });

  it("gives the light theme a palette whose every colour reads on its background, muted text dimmer; the dark keeps xterm's", async () => {
    document.documentElement.dataset.theme = "light";
    await mounted();
    const theme = term().options.theme as Record<string, string>;
    const names = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white", "brightBlack", "brightRed", "brightGreen", "brightYellow", "brightBlue", "brightMagenta", "brightCyan", "brightWhite"];
    for (const name of names) {
      expect(theme[name], name).toBe(LIGHT_ANSI[name as keyof typeof LIGHT_ANSI]);
      // Bright black is muted text (autosuggestions, secondary lines): it reads, but dimmer than the rest.
      if (name === "brightBlack") {
        expect(contrast(theme[name], "#fffdf8"), name).toBeGreaterThanOrEqual(3);
        expect(contrast(theme[name], "#fffdf8"), name).toBeLessThan(4.5);
      } else expect(contrast(theme[name], "#fffdf8"), name).toBeGreaterThanOrEqual(4.5);
    }
    delete document.documentElement.dataset.theme;
    mountLive(host(), { pid: 42 });
    expect((term().options.theme as Record<string, string>).white).toBeUndefined();
  });

  it("takes Maya's new theme while it is open, and stops following it once disposed", async () => {
    const handle = await mounted();
    const theme = () => term().options.theme as Record<string, string>;
    expect(theme().white).toBeUndefined();
    document.documentElement.dataset.theme = "light";
    await flush();
    expect(theme().white).toBe(LIGHT_ANSI.white);
    expect(theme().brightBlack).toBe(LIGHT_ANSI.brightBlack);
    delete document.documentElement.dataset.theme;
    await flush();
    expect(theme().white).toBeUndefined();
    handle.dispose();
    document.documentElement.dataset.theme = "light";
    await flush();
    expect(theme().white).toBeUndefined();
  });
});

function luminance(hex: string): number {
  const [red, green, blue] = [1, 3, 5].map((at) => parseInt(hex.slice(at, at + 2), 16) / 255).map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

/** WCAG contrast ratio of two #rrggbb colours. */
function contrast(first: string, second: string): number {
  const [lighter, darker] = [luminance(first), luminance(second)].sort((a, b) => b - a);
  return (lighter + 0.05) / (darker + 0.05);
}

describe("copyPasteKey", () => {
  const key = (key: string, mods: Partial<KeyboardEventInit> = {}) => new KeyboardEvent("keydown", { key, ...mods });

  it("maps the platform's copy and paste keys", () => {
    expect(copyPasteKey(key("c", { metaKey: true }), true)).toBe("copy");
    expect(copyPasteKey(key("v", { metaKey: true }), true)).toBe("paste");
    expect(copyPasteKey(key("c", { ctrlKey: true }), true)).toBeNull();
    expect(copyPasteKey(key("C", { ctrlKey: true, shiftKey: true }), false)).toBe("copy");
    expect(copyPasteKey(key("V", { ctrlKey: true, shiftKey: true }), false)).toBe("paste");
    expect(copyPasteKey(key("c", { ctrlKey: true }), false)).toBeNull();
    expect(copyPasteKey(key("c", { metaKey: true }), false)).toBeNull();
    expect(copyPasteKey(key("x", { metaKey: true }), true)).toBeNull();
  });
});

describe("mountLive on a remote session", () => {
  type Frame = { seq: number; kind: string; [field: string]: unknown };
  const send = (frame: Frame) => (FakeChannel.last as { onmessage: (m: Frame) => void }).onmessage(frame);
  const b64 = (s: string) => btoa(s);
  const snapshot = (text: string, rows = 40, cols = 120): Frame => ({ seq: 0, kind: "attached", snapshot: b64(text), rows, cols });
  /** What was drawn since the terminal was last reset. */
  const drawn = () => {
    const c = term().calls;
    return c.slice(c.lastIndexOf("reset") + 1).filter((x) => x.startsWith("write ")).map((x) => x.slice(6));
  };
  const resizes = () => invoke.mock.calls.filter((c) => /resize/.test(String(c[0])));

  /** Mounts a remote viewer whose attach resolves to `sub`, with its snapshot drawn. */
  async function remote(sub = 5) {
    invoke.mockImplementation((cmd: string) => (cmd === "live_attach_remote" ? Promise.resolve(sub) : Promise.resolve(null)));
    const handle = mountLive(host(), { session: "s1", remote: true });
    await flush();
    send(snapshot("hi"));
    return handle;
  }

  beforeEach(() => {
    document.body.innerHTML = "";
    invoke.mockReset();
    FakeFit.proposed = { rows: 40, cols: 120 };
    FakeTerminal.last = null;
    FakeChannel.last = null;
    observed = null;
    vi.useRealTimers();
  });

  it("attaches by session id through live_attach_remote and draws the snapshot at the host's size", async () => {
    await remote();
    expect(calls("live_attach_remote")).toEqual([["live_attach_remote", { sessionId: "s1", onFrame: FakeChannel.last }]]);
    expect(calls("native_attach")).toEqual([]);
    const c = term().calls;
    expect(c.indexOf("resize 40x120")).toBeLessThan(c.indexOf("reset"));
    expect(drawn()).toEqual(["hi"]);
    send({ seq: 1, kind: "output", data: b64("x") });
    expect(drawn()).toEqual(["hi", "x"]);
  });

  it("frames_that_arrive_before_the_snapshot_are_drawn_after_it", async () => {
    invoke.mockImplementation((cmd: string) => (cmd === "live_attach_remote" ? Promise.resolve(5) : Promise.resolve(null)));
    mountLive(host(), { session: "s1", remote: true });
    await flush();
    send({ seq: 2, kind: "output", data: b64("c") });
    send({ seq: 1, kind: "output", data: b64("b") });
    expect(term().text).toEqual([]);
    send(snapshot("a"));
    send({ seq: 3, kind: "output", data: b64("d") });
    expect(term().calls).toContain("resize 40x120");
    expect(drawn()).toEqual(["a", "b", "c", "d"]);
    expect(calls("live_detach_remote")).toEqual([]);
    expect(calls("live_attach_remote")).toHaveLength(1);
  });

  it("a_remote_viewer_never_resizes", async () => {
    await remote();
    FakeFit.proposed = { rows: 30, cols: 100 };
    observed!();
    await flush();
    send({ seq: 1, kind: "size", rows: 20, cols: 90 });
    observed!();
    await flush();
    expect(resizes()).toEqual([]);
    expect(term().calls).not.toContain("resize 30x100");
  });

  it("a size frame resizes the remote terminal", async () => {
    await remote();
    send({ seq: 1, kind: "size", rows: 30, cols: 100 });
    expect(term().calls[term().calls.length - 1]).toBe("resize 30x100");
    expect(resizes()).toEqual([]);
  });

  it("fits the font to the host's columns, on the snapshot, on a size frame and when the panel resizes", async () => {
    const h = host();
    let width = 1200;
    Object.defineProperty(h, "clientWidth", { get: () => width });
    invoke.mockImplementation((cmd: string) => (cmd === "live_attach_remote" ? Promise.resolve(5) : Promise.resolve(null)));
    mountLive(h, { session: "s1", remote: true });
    await flush();
    send(snapshot("hi"));
    expect(term().options.fontSize).toBe(12.5);
    send({ seq: 1, kind: "size", rows: 40, cols: 200 });
    expect(term().options.fontSize).toBe(fitFont(1200, 200));
    width = 600;
    observed!();
    expect(term().options.fontSize).toBe(8);
  });

  it("types keys into the remote terminal by its sub, and the extra keys follow the cursor mode", async () => {
    const handle = await remote(5);
    term().dataListener!("é");
    handle.press("ctrl-c");
    handle.press("up");
    term().modes.applicationCursorKeysMode = true;
    handle.press("up");
    expect(calls("live_input_remote").map((c) => c[1])).toEqual([
      { sub: 5, data: "w6k=" },
      { sub: 5, data: b64("\x03") },
      { sub: 5, data: b64("\x1b[A") },
      { sub: 5, data: b64("\x1bOA") },
    ]);
    expect(calls("native_input")).toEqual([]);
  });

  it("a local viewer's extra keys go to native_input", async () => {
    const handle = await mounted();
    handle.press("esc");
    expect(calls("native_input")).toEqual([["native_input", { term: 7, data: b64("\x1b") }]]);
  });

  it("detaches its sub on dispose, and at once when disposed before the attach resolves", async () => {
    const handle = await remote(5);
    handle.dispose();
    expect(calls("live_detach_remote")).toEqual([["live_detach_remote", { sub: 5 }]]);
    expect(term().disposed).toBe(true);

    invoke.mockReset();
    let resolve: (sub: number) => void = () => {};
    invoke.mockImplementation((cmd: string) => (cmd === "live_attach_remote" ? new Promise((r) => (resolve = r)) : Promise.resolve(null)));
    const early = mountLive(host(), { session: "s1", remote: true });
    early.dispose();
    resolve(6);
    await flush();
    expect(calls("live_detach_remote")).toEqual([["live_detach_remote", { sub: 6 }]]);
  });

  it("shows that the session ended and stops typing", async () => {
    await remote();
    send({ seq: 1, kind: "exit" });
    expect(term().text.join("")).toContain("[Session ended]");
    term().dataListener!("x");
    expect(calls("live_input_remote")).toEqual([]);
  });

  it("shows an error frame and does not attach again", async () => {
    vi.useFakeTimers();
    invoke.mockImplementation((cmd: string) => (cmd === "live_attach_remote" ? Promise.resolve(5) : Promise.resolve(null)));
    const handle = mountLive(host(), { session: "s1", remote: true });
    await vi.advanceTimersByTimeAsync(0);
    send({ seq: 0, kind: "error", message: "That Maya cannot share a live terminal. Update it." });
    expect(term().text.join("")).toContain("[That Maya cannot share a live terminal. Update it.]");
    await vi.advanceTimersByTimeAsync(REATTACH_MS * 3);
    expect(calls("live_attach_remote")).toHaveLength(1);
    term().dataListener!("x");
    handle.dispose();
    // The error ended the sub: nothing is typed into it or detached from it.
    expect(calls("live_input_remote")).toEqual([]);
    expect(calls("live_detach_remote")).toEqual([]);
  });

  it("shows why the first attach was refused and does not try again", async () => {
    vi.useFakeTimers();
    invoke.mockImplementation((cmd: string) => (cmd === "live_attach_remote" ? Promise.reject("That Maya is not connected.") : Promise.resolve(null)));
    mountLive(host(), { session: "s1", remote: true });
    await vi.advanceTimersByTimeAsync(0);
    expect(term().text.join("")).toContain("[That Maya is not connected.]");
    await vi.advanceTimersByTimeAsync(REATTACH_MS * 3);
    expect(calls("live_attach_remote")).toHaveLength(1);
  });

  it("a_lost_viewer_attaches_again", async () => {
    vi.useFakeTimers();
    invoke.mockImplementation((cmd: string) => (cmd === "live_attach_remote" ? Promise.resolve(5) : Promise.resolve(null)));
    const handle = mountLive(host(), { session: "s1", remote: true });
    await vi.advanceTimersByTimeAsync(0);
    send(snapshot("hi"));
    send({ seq: 0, kind: "lost" });
    expect(term().text[term().text.length - 1]).toContain("[Connection lost. Reconnecting…]");
    // The lost sub is gone: nothing is typed into it or detached from it.
    term().dataListener!("x");
    expect(calls("live_input_remote")).toEqual([]);
    await vi.advanceTimersByTimeAsync(REATTACH_MS - 1);
    expect(calls("live_attach_remote")).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(calls("live_attach_remote")).toHaveLength(2);
    // Each try has its own channel: the old one is no longer heard.
    expect(calls("live_attach_remote")[1][1].onFrame).not.toBe(calls("live_attach_remote")[0][1].onFrame);
    expect(calls("live_detach_remote")).toEqual([]);
    send(snapshot("back"));
    expect(drawn()).toEqual(["back"]);
    await vi.advanceTimersByTimeAsync(REATTACH_MS * 3);
    expect(calls("live_attach_remote")).toHaveLength(2);
    term().dataListener!("y");
    expect(calls("live_input_remote")).toEqual([["live_input_remote", { sub: 5, data: b64("y") }]]);
    handle.dispose();
  });

  it("a lost that comes before the attach resolves still attaches again, never touching the lost sub", async () => {
    vi.useFakeTimers();
    let resolve: (sub: number) => void = () => {};
    invoke.mockImplementation((cmd: string) => (cmd === "live_attach_remote" ? new Promise((r) => (resolve = r)) : Promise.resolve(null)));
    const handle = mountLive(host(), { session: "s1", remote: true });
    send({ seq: 0, kind: "lost" });
    resolve(5);
    await vi.advanceTimersByTimeAsync(0);
    term().dataListener!("x");
    expect(calls("live_input_remote")).toEqual([]);
    await vi.advanceTimersByTimeAsync(REATTACH_MS);
    expect(calls("live_attach_remote")).toHaveLength(2);
    resolve(6);
    await vi.advanceTimersByTimeAsync(0);
    handle.dispose();
    expect(calls("live_detach_remote")).toEqual([["live_detach_remote", { sub: 6 }]]);
  });

  it("a refused try after a lost keeps trying, quietly, until the snapshot comes", async () => {
    vi.useFakeTimers();
    let up = false;
    invoke.mockImplementation((cmd: string) => (cmd === "live_attach_remote" ? (up ? Promise.resolve(7) : Promise.reject("That Maya is not connected.")) : Promise.resolve(null)));
    up = true;
    mountLive(host(), { session: "s1", remote: true });
    await vi.advanceTimersByTimeAsync(0);
    send(snapshot("hi"));
    up = false;
    send({ seq: 1, kind: "output", data: b64("x") });
    send({ seq: 0, kind: "lost" });
    await vi.advanceTimersByTimeAsync(REATTACH_MS * 2);
    expect(calls("live_attach_remote")).toHaveLength(3);
    expect(term().text.join("")).not.toContain("not connected");
    expect(term().text.filter((t) => t.includes("Reconnecting"))).toHaveLength(1);
    up = true;
    await vi.advanceTimersByTimeAsync(REATTACH_MS);
    expect(calls("live_attach_remote")).toHaveLength(4);
    send(snapshot("back"));
    await vi.advanceTimersByTimeAsync(REATTACH_MS * 3);
    expect(calls("live_attach_remote")).toHaveLength(4);
    expect(drawn()).toEqual(["back"]);
  });

  it("a gap in the numbering attaches again", async () => {
    let next = 5;
    invoke.mockImplementation((cmd: string) => (cmd === "live_attach_remote" ? Promise.resolve(next++) : Promise.resolve(null)));
    mountLive(host(), { session: "s1", remote: true });
    await flush();
    send(snapshot("hi"));
    send({ seq: 1, kind: "output", data: b64("a") });
    const first = FakeChannel.last;
    send({ seq: 3, kind: "output", data: b64("c") });
    expect(drawn()).toEqual(["hi", "a"]);
    expect(calls("live_detach_remote")).toEqual([["live_detach_remote", { sub: 5 }]]);
    expect(calls("live_attach_remote")).toHaveLength(2);
    expect(term().text.join("")).not.toContain("Reconnecting");
    await flush();
    // The old channel is no longer heard; the new one draws afresh.
    (first as { onmessage: (m: Frame) => void }).onmessage({ seq: 4, kind: "output", data: b64("old") });
    send(snapshot("fresh"));
    send({ seq: 1, kind: "output", data: b64("b") });
    expect(drawn()).toEqual(["fresh", "b"]);
    term().dataListener!("k");
    expect(calls("live_input_remote")).toEqual([["live_input_remote", { sub: 6, data: b64("k") }]]);
  });
});

describe("the extra keys", () => {
  it("keyBytes follows the cursor mode", () => {
    expect(keyBytes("up", false)).toBe("\x1b[A");
    expect(keyBytes("up", true)).toBe("\x1bOA");
    expect(keyBytes("down", false)).toBe("\x1b[B");
    expect(keyBytes("right", true)).toBe("\x1bOC");
    expect(keyBytes("left", false)).toBe("\x1b[D");
    expect(keyBytes("esc", true)).toBe("\x1b");
    expect(keyBytes("tab", false)).toBe("\t");
    expect(keyBytes("ctrl-c", true)).toBe("\x03");
    expect(EXTRA_KEYS.map(([, l]) => l)).toEqual(["Esc", "Tab", "Ctrl+C", "←", "↑", "↓", "→"]);
    expect(EXTRA_KEYS.map(([k]) => k)).toEqual(["esc", "tab", "ctrl-c", "left", "up", "down", "right"]);
  });

  it("fitFont fits the columns between 8 and 12.5", () => {
    expect(fitFont(1200, 120)).toBe(12.5);
    expect(fitFont(400, 120)).toBe(8);
    expect(fitFont(800, 120)).toBe(11);
  });
});
