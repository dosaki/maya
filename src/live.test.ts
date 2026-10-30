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
    write(data: string | Uint8Array) {
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

import { copyPasteKey, mountLive } from "./live";

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
  const handle = mountLive(host(), 42);
  await flush();
  return handle;
}

describe("mountLive", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    invoke.mockReset();
    FakeFit.proposed = { rows: 40, cols: 120 };
    FakeTerminal.last = null;
    observed = null;
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

  it("resizes to the attached size before writing the snapshot", async () => {
    FakeFit.proposed = { rows: 30, cols: 100 };
    await mounted();
    const c = term().calls;
    const sized = c.indexOf("resize 40x120");
    const snapshot = c.indexOf("write hi");
    expect(sized).toBeGreaterThan(-1);
    expect(sized).toBeLessThan(snapshot);
    // Then fitted to the panel, and the backend told.
    expect(c.lastIndexOf("resize 30x100")).toBeGreaterThan(snapshot);
  });

  it("queues frames that arrive before the attach resolves and writes them after the snapshot", async () => {
    let resolve: (a: typeof attached) => void = () => {};
    invoke.mockImplementation((cmd: string) => (cmd === "native_attach" ? new Promise((r) => (resolve = r)) : Promise.resolve(null)));
    mountLive(host(), 42);
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
    expect(calls("native_resize")).toEqual([["native_resize", { term: 7, rows: 30, cols: 100 }]]);
    // The same size again is not sent twice.
    observed!();
    expect(calls("native_resize")).toHaveLength(1);
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
    mountLive(host(), 42);
    channel().onmessage({ kind: "exit" });
    resolve(attached);
    await flush();
    expect(term().text[0]).toBe("hi");
    expect(term().text.slice(1).join("")).toContain("[Session ended]");
  });

  it("shows why it could not attach", async () => {
    invoke.mockImplementation((cmd: string) => (cmd === "native_attach" ? Promise.reject("This session is not running in Maya.") : Promise.resolve(null)));
    mountLive(host(), 42);
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
    const handle = mountLive(host(), 42);
    handle.dispose();
    resolve(attached);
    await flush();
    expect(calls("native_detach")).toEqual([["native_detach", { term: 7, viewer: 1 }]]);
    expect(term().text).toEqual([]);
  });

  it("copies the selection and pastes the clipboard on the platform's keys, and xterm sends neither", async () => {
    const writeText = vi.fn(() => Promise.resolve());
    const readText = vi.fn(() => Promise.resolve("pasted"));
    Object.defineProperty(navigator, "clipboard", { value: { writeText, readText }, configurable: true });
    await mounted();
    term().selection = "chosen";
    const mac = /Macintosh/.test(navigator.userAgent);
    const mod = mac ? { metaKey: true } : { ctrlKey: true, shiftKey: true };
    expect(term().keyHandler!(new KeyboardEvent("keydown", { key: mac ? "c" : "C", ...mod }))).toBe(false);
    expect(writeText).toHaveBeenCalledWith("chosen");
    expect(term().keyHandler!(new KeyboardEvent("keydown", { key: mac ? "v" : "V", ...mod }))).toBe(false);
    await flush();
    expect(term().pasted).toEqual(["pasted"]);
    expect(term().keyHandler!(new KeyboardEvent("keydown", { key: "a" }))).toBe(true);
  });
});

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
