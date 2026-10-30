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

/** The terminal look's colours, from the CSS variables the viewer sets. */
function themeOf(host: HTMLElement): ITheme {
  const css = getComputedStyle(host);
  const v = (name: string) => css.getPropertyValue(name).trim() || undefined;
  return { background: v("--term-bg"), foreground: v("--term-fg"), cursor: v("--term-cursor"), selectionBackground: v("--term-selection") };
}

/** A dim line of Maya's own under the program's output. */
const note = (text: string) => `\r\n\x1b[2m${text}\x1b[0m`;

/**
 * Opens a terminal in `host` and attaches it to the Native terminal `pid`
 * runs in. The snapshot is drawn at the size it was taken at, then the
 * terminal fits `host` and the program is told the new size.
 */
export function mountLive(host: HTMLElement, pid: number): LiveHandle {
  const term = new Terminal({ scrollback: 10000, cursorBlink: true, allowProposedApi: false, theme: themeOf(host), fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace", fontSize: 12.5 });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(host);

  let attached: Attached | null = null;
  let ended = false;
  let disposed = false;
  // Frames that arrive before the snapshot is drawn wait for it.
  let queued: NativeFrame[] | null = [];
  // The size the program was last told.
  let told = { rows: 0, cols: 0 };

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
    if (disposed) return;
    fit.fit();
    if (!attached || ended || (term.rows === told.rows && term.cols === told.cols)) return;
    told = { rows: term.rows, cols: term.cols };
    void invoke("native_resize", { term: attached.term, rows: term.rows, cols: term.cols }).catch(() => undefined);
  };

  const channel = new Channel<NativeFrame>();
  channel.onmessage = (frame) => {
    if (disposed) return;
    if (queued) queued.push(frame);
    else apply(frame);
  };
  invoke<Attached>("native_attach", { pid, onFrame: channel })
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
      term.write(decode(a.snapshot));
      const waiting = queued ?? [];
      queued = null;
      for (const frame of waiting) apply(frame);
      refit();
    })
    .catch((e) => {
      if (disposed) return;
      queued = null;
      ended = true;
      term.write(note(String(e)));
    });

  term.onData((data) => send(new TextEncoder().encode(data)));
  // Mouse reports in the legacy encoding: one byte per character.
  term.onBinary((data) => send(Uint8Array.from(data, (c) => c.charCodeAt(0) & 0xff)));

  const mac = isMac();
  term.attachCustomKeyEventHandler((ev) => {
    const key = copyPasteKey(ev, mac);
    if (!key) return true;
    if (ev.type === "keydown") {
      // Neither the webview's own paste nor the key itself reaches the program.
      ev.preventDefault();
      if (key === "copy") {
        const text = term.getSelection();
        if (text) void navigator.clipboard.writeText(text).catch(() => undefined);
      } else {
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
  refit();

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
