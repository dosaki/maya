import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** One line of Maya's log, as `log.rs` emits it. */
export interface LogLine {
  /** Unix time in milliseconds. */
  at: number;
  source: string;
  text: string;
}

export interface DebugModel {
  lines: LogLine[];
  /** Sources whose lines are hidden; empty shows everything. */
  hidden: Set<string>;
  /** Where this launch's log file is. */
  path: string;
}

export interface DebugHandlers {
  onToggle(source: string): void;
  onClear(): void;
  onCopy(): void;
}

/** The lines whose source is not hidden. */
export function visibleLines(lines: LogLine[], hidden: Set<string>): LogLine[] {
  return lines.filter((l) => !hidden.has(l.source));
}

/** `HH:MM:SS.mmm` in local time. */
export function clock(at: number): string {
  const d = new Date(at);
  const p = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`;
}

/** The log as text, one record per line, the way the file has it. */
export function asText(lines: LogLine[]): string {
  return lines.map((l) => `${clock(l.at)} ${l.source}: ${l.text.replace(/\n/g, "\n    ")}`).join("\n");
}

export function renderDebug(m: DebugModel, h: DebugHandlers): HTMLElement {
  const root = document.createElement("div");
  root.className = "debug";

  const bar = document.createElement("div");
  bar.className = "debug__bar";
  const sources = [...new Set(m.lines.map((l) => l.source))].sort();
  for (const s of sources) {
    const chip = document.createElement("button");
    chip.type = "button";
    chip.className = "debug__chip" + (m.hidden.has(s) ? " debug__chip--off" : "");
    chip.textContent = s;
    chip.title = m.hidden.has(s) ? `Show ${s} lines` : `Hide ${s} lines`;
    chip.addEventListener("click", () => h.onToggle(s));
    bar.append(chip);
  }
  const spacer = document.createElement("span");
  spacer.className = "debug__spacer";
  const copy = document.createElement("button");
  copy.type = "button";
  copy.dataset.action = "log-copy";
  copy.textContent = "Copy";
  copy.title = "Copy the shown lines";
  copy.addEventListener("click", () => h.onCopy());
  const clear = document.createElement("button");
  clear.type = "button";
  clear.dataset.action = "log-clear";
  clear.textContent = "Clear";
  clear.title = "Clear the view; the file keeps everything";
  clear.addEventListener("click", () => h.onClear());
  bar.append(spacer, copy, clear);
  root.append(bar);

  const list = document.createElement("div");
  list.className = "debug__lines";
  const shown = visibleLines(m.lines, m.hidden);
  if (shown.length === 0) {
    const empty = document.createElement("div");
    empty.className = "debug__empty";
    empty.textContent = m.lines.length === 0 ? "Nothing logged yet." : "Every source is hidden.";
    list.append(empty);
  }
  for (const l of shown) {
    const row = document.createElement("div");
    row.className = `debug__line debug__line--${l.source}`;
    const time = document.createElement("span");
    time.className = "debug__time";
    time.textContent = clock(l.at);
    const source = document.createElement("span");
    source.className = "debug__source";
    source.textContent = l.source;
    const text = document.createElement("span");
    text.className = "debug__text";
    text.textContent = l.text;
    row.append(time, source, text);
    list.append(row);
  }
  root.append(list);

  const path = document.createElement("div");
  path.className = "debug__path";
  path.textContent = m.path ? `This launch's log: ${m.path}` : "";
  root.append(path);
  return root;
}

/** Mounts the Debug pane and keeps it current with the `log` event. */
export async function initDebug(): Promise<void> {
  const pane = document.getElementById("debug");
  if (!pane) return;
  const model: DebugModel = { lines: [], hidden: new Set(), path: "" };
  const handlers: DebugHandlers = {
    onToggle: (s) => {
      if (model.hidden.has(s)) model.hidden.delete(s);
      else model.hidden.add(s);
      paint();
    },
    onClear: () => {
      model.lines = [];
      void invoke("log_clear");
      paint();
    },
    onCopy: () => {
      void navigator.clipboard?.writeText(asText(visibleLines(model.lines, model.hidden)));
    },
  };
  const paint = () => {
    const list = pane.querySelector<HTMLElement>(".debug__lines");
    const atBottom = !list || list.scrollTop + list.clientHeight >= list.scrollHeight - 8;
    pane.replaceChildren(renderDebug(model, handlers));
    const fresh = pane.querySelector<HTMLElement>(".debug__lines");
    if (fresh && atBottom) fresh.scrollTop = fresh.scrollHeight;
  };
  await listen<LogLine>("log", (e) => {
    model.lines.push(e.payload);
    if (model.lines.length > 500) model.lines.shift();
    if (!pane.hidden) paint();
  });
  // A repaint when the tab is shown, so lines that arrived while hidden appear.
  new MutationObserver(() => {
    if (!pane.hidden) paint();
  }).observe(pane, { attributes: true, attributeFilter: ["hidden"] });
  const [lines, path] = await Promise.all([invoke<LogLine[]>("log_lines"), invoke<string>("log_path")]);
  model.lines = lines;
  model.path = path;
  paint();
}
