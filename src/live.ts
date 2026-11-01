// The live terminal of a Native session: an xterm.js terminal attached to
// the pseudo-terminal Maya hosts, typed into directly as in a terminal app.

// xterm's own styles come with this module: only a page that opens a live terminal loads them.
import "@xterm/xterm/css/xterm.css";
import { Channel, invoke } from "@tauri-apps/api/core";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal, type ITheme } from "@xterm/xterm";
import { type ExtraKey, keyBytes } from "./livekeys";
import { isMac } from "./platform";

export { EXTRA_KEYS, type ExtraKey, keyBytes } from "./livekeys";

export interface LiveHandle {
  dispose(): void;
  focus(): void;
  /** Types one of the extra keys, as the program's cursor mode wants it. */
  press(key: ExtraKey): void;
}

/** What the backend streams to a viewer, bytes as base64. */
type NativeFrame = { kind: "output"; data: string } | { kind: "size"; rows: number; cols: number } | { kind: "exit" };

/**
 * What a remote terminal streams, bytes as base64: the snapshot is `seq` 0
 * and every later event the next number. `lost`, and an `error` sent
 * instead of a snapshot, carry 0.
 */
type TermFrame =
  | { seq: number; kind: "attached"; snapshot: string; rows: number; cols: number }
  | { seq: number; kind: "output"; data: string }
  | { seq: number; kind: "size"; rows: number; cols: number }
  | { seq: number; kind: "exit" }
  | { seq: number; kind: "error"; message: string }
  | { seq: number; kind: "lost" };

/**
 * The terminal to attach to: the one a session's process runs in, one by
 * its id, or a session's on another Maya, reached through the main.
 */
export type LiveTarget = { pid: number } | { term: number } | { session: string; remote: true };

/** How long a lost remote viewer waits before it tries to attach again. */
export const REATTACH_MS = 2000;

/** What `native_attach` hands back: the screen so far, at the size it was drawn for. */
interface Attached {
  term: number;
  viewer: number;
  snapshot: string;
  rows: number;
  cols: number;
}

/**
 * Which of the platform's clipboard keys `ev` is: Cmd+C and Cmd+V on macOS,
 * Ctrl+Shift+C and Ctrl+Shift+V elsewhere. Plain Ctrl+C is the program's.
 */
export function copyPasteKey(ev: KeyboardEvent, mac: boolean): "copy" | "paste" | null {
  const mods = mac ? ev.metaKey && !ev.ctrlKey && !ev.altKey && !ev.shiftKey : ev.ctrlKey && ev.shiftKey && !ev.altKey && !ev.metaKey;
  if (!mods) return null;
  const key = ev.key.toLowerCase();
  return key === "c" ? "copy" : key === "v" ? "paste" : null;
}

function decode(b64: string): Uint8Array {
  return Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
}

function encode(bytes: Uint8Array): string {
  return btoa(Array.from(bytes, (b) => String.fromCharCode(b)).join(""));
}

/**
 * The 16 ANSI colours on Maya's light background (#fffdf8): xterm's own are
 * made for a dark one, where its white and yellow would not read. Bright
 * black is muted text, so it stays a mid-grey (3:1) while the rest read at 4.5:1.
 */
export const LIGHT_ANSI = {
  black: "#231f14",
  red: "#b3261e",
  green: "#2e6b30",
  yellow: "#7a5a00",
  blue: "#1d56b0",
  magenta: "#8a3aa3",
  cyan: "#0e6a73",
  white: "#5f5847",
  brightBlack: "#8a8170",
  brightRed: "#a1200f",
  brightGreen: "#275c29",
  brightYellow: "#6b4f00",
  brightBlue: "#174a99",
  brightMagenta: "#77308d",
  brightCyan: "#0b5b63",
  brightWhite: "#3d3729",
} satisfies ITheme;

/** The terminal look's colours, from the CSS variables the viewer sets, with the light palette in the light theme. */
function themeOf(host: HTMLElement): ITheme {
  const css = getComputedStyle(host);
  const v = (name: string) => css.getPropertyValue(name).trim() || undefined;
  const own = { background: v("--term-bg"), foreground: v("--term-fg"), cursor: v("--term-cursor"), selectionBackground: v("--term-selection") };
  return document.documentElement.dataset.theme === "light" ? { ...LIGHT_ANSI, ...own } : own;
}

/** A dim line of Maya's own under the program's output. */
const note = (text: string) => `\r\n\x1b[2m${text}\x1b[0m`;

/**
 * The font size, between 8 and 12.5 px in half pixels, at which `cols`
 * columns fit `width` pixels; past 8 px the terminal scrolls sideways.
 */
export function fitFont(width: number, cols: number): number {
  return Math.max(8, Math.min(12.5, Math.floor((width / (cols * 0.6)) * 2) / 2));
}

/** A viewer's way to the terminal it shows. */
interface Link {
  send(bytes: Uint8Array): void;
  /** The viewer's size changed. */
  refit(): void;
  detach(): void;
}

/**
 * Opens a terminal in `host` and attaches it to the Native terminal
 * `target` names. A local snapshot is drawn at the size it was taken at,
 * then the terminal fits `host` and the program is told the new size. A
 * remote one keeps the host's size and fits its font to `host` instead.
 */
export function mountLive(host: HTMLElement, target: LiveTarget): LiveHandle {
  const term = new Terminal({ scrollback: 10000, cursorBlink: true, allowProposedApi: false, theme: themeOf(host), fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace", fontSize: 12.5 });
  let link: Link;
  if ("remote" in target) {
    term.open(host);
    link = attachRemote(term, host, target.session);
  } else {
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    link = attachLocal(term, fit, target);
  }
  let disposed = false;
  const send = (bytes: Uint8Array) => link.send(bytes);

  term.onData((data) => send(new TextEncoder().encode(data)));
  // Mouse reports in the legacy encoding: one byte per character.
  term.onBinary((data) => send(Uint8Array.from(data, (c) => c.charCodeAt(0) & 0xff)));

  const mac = isMac();
  term.attachCustomKeyEventHandler((ev) => {
    const key = copyPasteKey(ev, mac);
    if (!key) return true;
    // On macOS the webview's own paste (not prevented) reaches xterm's
    // textarea, and xterm pastes it, bracketed when the program asks;
    // reading the clipboard instead would show WebKit's Paste prompt.
    if (key === "paste" && mac) return false;
    if (ev.type === "keydown") {
      // Neither the webview's own copy or paste nor the key itself reaches the program.
      ev.preventDefault();
      if (key === "copy") {
        const text = term.getSelection();
        if (text) void navigator.clipboard.writeText(text).catch(() => undefined);
      } else {
        // Elsewhere Ctrl+V is the program's (^V), and not every webview pastes
        // on Ctrl+Shift+V: read the clipboard, the webview's own paste prevented.
        void navigator.clipboard
          .readText()
          .then((text) => {
            if (!disposed && text) term.paste(text);
          })
          .catch(() => undefined);
      }
    }
    return false;
  });

  const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => link.refit());
  observer?.observe(host);
  // Maya's theme changes on the page's root: the open terminal takes its colours at once.
  const restyle = new MutationObserver(() => {
    if (!disposed) term.options.theme = themeOf(host);
  });
  restyle.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });

  return {
    dispose() {
      if (disposed) return;
      disposed = true;
      observer?.disconnect();
      restyle.disconnect();
      link.detach();
      term.dispose();
    },
    focus() {
      term.focus();
    },
    press(key) {
      send(new TextEncoder().encode(keyBytes(key, term.modes.applicationCursorKeysMode)));
    },
  };
}

/** Attaches `term` to a Native terminal on this machine, which it sizes. */
function attachLocal(term: Terminal, fit: FitAddon, target: { pid: number } | { term: number }): Link {
  let attached: Attached | null = null;
  let ended = false;
  let disposed = false;
  // Fitting waits for the snapshot to be parsed: xterm parses writes later
  // but resizes at once, so an earlier fit would lay the snapshot out wrong.
  let ready = false;
  // Frames that arrive before the snapshot is drawn wait for it.
  let queued: NativeFrame[] | null = [];
  // The size the program was last told.
  let told = { rows: 0, cols: 0 };
  // Resizes run off the main thread: each waits for the one before, so the last size wins.
  let resizing: Promise<unknown> = Promise.resolve();

  const apply = (frame: NativeFrame) => {
    if (frame.kind === "output") term.write(decode(frame.data));
    // The size is for viewers on other machines: this one sizes the terminal itself.
    else if (frame.kind === "size") return;
    else if (!ended) {
      ended = true;
      term.write(note("[Session ended]"));
    }
  };
  const send = (bytes: Uint8Array) => {
    if (!attached || ended || disposed) return;
    void invoke("native_input", { term: attached.term, data: encode(bytes) }).catch(() => undefined);
  };
  const refit = () => {
    if (disposed || !ready) return;
    fit.fit();
    if (!attached || ended || (term.rows === told.rows && term.cols === told.cols)) return;
    told = { rows: term.rows, cols: term.cols };
    const size = { term: attached.term, rows: term.rows, cols: term.cols };
    resizing = resizing.then(() => invoke("native_resize", size)).catch(() => undefined);
  };

  const channel = new Channel<NativeFrame>();
  channel.onmessage = (frame) => {
    if (disposed) return;
    if (queued) queued.push(frame);
    else apply(frame);
  };
  const attach = "pid" in target ? invoke<Attached>("native_attach", { pid: target.pid, onFrame: channel }) : invoke<Attached>("native_attach_term", { term: target.term, onFrame: channel });
  attach
    .then((a) => {
      if (disposed) {
        void invoke("native_detach", { term: a.term, viewer: a.viewer }).catch(() => undefined);
        return;
      }
      attached = a;
      told = { rows: a.rows, cols: a.cols };
      // The snapshot's layout is the program's size: draw it at that size.
      term.resize(a.cols, a.rows);
      term.reset();
      term.write(decode(a.snapshot), () => {
        ready = true;
        refit();
      });
      // Writes stay in order: what came meanwhile follows the snapshot.
      const waiting = queued ?? [];
      queued = null;
      for (const frame of waiting) apply(frame);
    })
    .catch((e) => {
      if (disposed) return;
      queued = null;
      ended = true;
      ready = true;
      refit();
      term.write(note(String(e)));
    });

  return {
    send,
    refit,
    detach() {
      disposed = true;
      if (attached) void invoke("native_detach", { term: attached.term, viewer: attached.viewer }).catch(() => undefined);
    },
  };
}

/**
 * Attaches `term` to the Native terminal of `session` on another Maya. It
 * draws the host's size and never resizes it: the font fits the columns to
 * `host` instead. A lost link tries again every `REATTACH_MS`, and a gap in
 * the numbering takes a fresh snapshot.
 */
function attachRemote(term: Terminal, host: HTMLElement, session: string): Link {
  // One try at attaching: its sub once the attach resolves, and whether
  // that sub is over (lost, ended or detached), never to be touched again.
  type Try = { sub: number | null; over: boolean; first: boolean };
  let now: Try | null = null;
  let tries = 0;
  // The number of the frame last drawn: -1 until the snapshot.
  let last = -1;
  // Frames numbered after the snapshot that came before it.
  let held: TermFrame[] = [];
  let ended = false;
  let disposed = false;
  let lost = false;
  let retry: ReturnType<typeof setTimeout> | undefined;

  const refit = () => {
    if (disposed) return;
    // The terminal's own padding is not room for columns.
    const css = term.element ? getComputedStyle(term.element) : null;
    const padding = css ? (parseFloat(css.paddingLeft) || 0) + (parseFloat(css.paddingRight) || 0) : 0;
    const size = fitFont(host.clientWidth - padding, term.cols);
    if (term.options.fontSize !== size) term.options.fontSize = size;
  };
  const detach = (t: Try) => {
    if (t.sub === null || t.over) return;
    t.over = true;
    void invoke("live_detach_remote", { sub: t.sub }).catch(() => undefined);
  };
  const end = (text: string) => {
    ended = true;
    clearTimeout(retry);
    if (now) now.over = true;
    term.write(note(text));
  };
  const lose = () => {
    if (!lost) term.write(note("[Connection lost. Reconnecting…]"));
    lost = true;
    if (retry === undefined)
      retry = setTimeout(() => {
        retry = undefined;
        attach();
      }, REATTACH_MS);
  };
  // A numbered frame after the snapshot; one out of turn takes a fresh snapshot.
  const draw = (frame: TermFrame) => {
    if (frame.seq !== last + 1) {
      if (now) detach(now);
      attach();
      return;
    }
    last = frame.seq;
    if (frame.kind === "output") term.write(decode(frame.data));
    else if (frame.kind === "size") {
      term.resize(frame.cols, frame.rows);
      refit();
    } else if (frame.kind === "exit") end("[Session ended]");
  };
  const take = (t: Try, frame: TermFrame) => {
    // A try given up, or a sub that is over, is no longer heard.
    if (disposed || ended || t !== now || t.over) return;
    if (frame.kind === "lost") {
      t.over = true;
      lose();
    } else if (frame.kind === "error") end(`[${frame.message}]`);
    else if (frame.kind === "attached") {
      lost = false;
      last = 0;
      // The snapshot's layout is the host's size: draw it at that size.
      term.resize(frame.cols, frame.rows);
      term.reset();
      refit();
      term.write(decode(frame.snapshot));
      // What came early follows the snapshot, in order.
      const early = held.sort((a, b) => a.seq - b.seq);
      held = [];
      for (const f of early) {
        if (f.seq > last) draw(f);
        if (t !== now || ended) break;
      }
    } else if (last < 0) held.push(frame);
    else draw(frame);
  };
  const attach = () => {
    if (disposed || ended) return;
    const t: Try = { sub: null, over: false, first: tries++ === 0 };
    now = t;
    last = -1;
    held = [];
    const channel = new Channel<TermFrame>();
    channel.onmessage = (frame) => take(t, frame);
    invoke<number>("live_attach_remote", { sessionId: session, onFrame: channel })
      .then((sub) => {
        t.sub = sub;
        if (disposed || t !== now) detach(t);
      })
      .catch((e) => {
        if (disposed || ended || t !== now || t.over) return;
        t.over = true;
        // Refused at first, it says why; refused on a later try, the link is still down.
        if (t.first) end(`[${String(e)}]`);
        else lose();
      });
  };
  attach();

  return {
    send(bytes) {
      if (disposed || ended || !now || now.sub === null || now.over || last < 0) return;
      void invoke("live_input_remote", { sub: now.sub, data: encode(bytes) }).catch(() => undefined);
    },
    refit,
    detach() {
      disposed = true;
      clearTimeout(retry);
      if (now) detach(now);
    },
  };
}
