// The live terminal of a Native session: an xterm.js terminal attached to
// the pseudo-terminal Maya hosts, typed into directly as in a terminal app.

import { Channel, invoke } from "@tauri-apps/api/core";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal, type ITheme } from "@xterm/xterm";
import { isMac } from "./platform";

export interface LiveHandle {
  dispose(): void;
  focus(): void;
}

/** What the backend streams to a viewer, bytes as base64. */
type NativeFrame = { kind: "output"; data: string } | { kind: "exit" };

/** The terminal to attach to: the one a session's process runs in, or one by its id. */
export type LiveTarget = { pid: number } | { term: number };

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
 * made for a dark one, where its white and yellow would not read.
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
  brightBlack: "#4a4335",
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
 * Opens a terminal in `host` and attaches it to the Native terminal
 * `target` names. The snapshot is drawn at the size it was taken at, then
 * the terminal fits `host` and the program is told the new size.
 */
export function mountLive(host: HTMLElement, target: LiveTarget): LiveHandle {
  const term = new Terminal({ scrollback: 10000, cursorBlink: true, allowProposedApi: false, theme: themeOf(host), fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace", fontSize: 12.5 });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(host);

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

  const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => refit());
  observer?.observe(host);

  return {
    dispose() {
      if (disposed) return;
      disposed = true;
      observer?.disconnect();
      if (attached) void invoke("native_detach", { term: attached.term, viewer: attached.viewer }).catch(() => undefined);
      term.dispose();
    },
    focus() {
      term.focus();
    },
  };
}
