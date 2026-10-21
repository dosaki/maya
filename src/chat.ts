import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Dot = "idle" | "working" | "waiting" | "unread";

export interface ChatTurn {
  who: "user" | "maya" | string;
  text: string;
  at: number;
  via: "voice" | "chat" | "phone" | "maya";
}

export interface BrainStatus {
  dot: Dot;
  /** The batch waiting in the action window; `until` is null until her announcement has been said. */
  pending: { say: string; until: number | null } | null;
  job: string | null;
  ready: boolean;
}

export interface ChatModel {
  turns: ChatTurn[];
  status: BrainStatus;
  draft: string;
}

export interface ChatHandlers {
  onSend(text: string): void;
  onCancel(): void;
  onStop(): void;
}

const IDLE: BrainStatus = { dot: "idle", pending: null, job: null, ready: false };

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

/** Whole seconds left on a pending batch, never below zero; null while its clock has not started. */
export function secondsLeft(until: number | null, now = Date.now()): number | null {
  return until === null ? null : Math.max(0, Math.ceil((until - now) / 1000));
}

/** The conversation, the pending box, the input and Stop. Her text is set as text, never as markup. */
export function renderChat(m: ChatModel, h: ChatHandlers): HTMLElement {
  const root = el("div", "chat");
  const log = el("div", "chat__log");
  log.setAttribute("role", "log");
  for (const t of m.turns) {
    const turn = el("div", `chat__turn chat__turn--${t.who === "user" ? "user" : "maya"}`, t.text);
    turn.title = t.via;
    log.append(turn);
  }
  root.append(log);

  const p = m.status.pending;
  if (p) {
    const box = el("div", "chat__pending");
    const left = secondsLeft(p.until);
    box.append(el("span", "chat__pending-say", p.say), el("span", "chat__pending-left", left === null ? "" : `${left}s`));
    const cancel = el("button", "chat__cancel", "Cancel");
    cancel.type = "button";
    cancel.dataset.action = "window-cancel";
    cancel.addEventListener("click", () => h.onCancel());
    box.append(cancel);
    root.append(box);
  }

  const bar = el("div", "chat__bar");
  const input = el("textarea", "chat__input");
  input.name = "chat";
  input.rows = 2;
  input.placeholder = "Tell Maya what to do";
  input.value = m.draft;
  input.addEventListener("keydown", (ev) => {
    if (ev.key !== "Enter" || ev.shiftKey || ev.isComposing) return;
    ev.preventDefault();
    const text = input.value.trim();
    if (!text) return;
    input.value = "";
    h.onSend(text);
  });
  bar.append(input);
  if (m.status.dot === "working") {
    const stop = el("button", "chat__stop", m.status.job ? `Stop (${m.status.job})` : "Stop");
    stop.type = "button";
    stop.dataset.action = "brain-stop";
    stop.addEventListener("click", () => h.onStop());
    bar.append(stop);
  }
  root.append(bar);
  return root;
}

/** Wires the Chat pane and the dot on the Chat tab: history, live turns and status, seen on open. */
export function initChat(): void {
  const pane = document.getElementById("chat");
  let turns: ChatTurn[] = [];
  let status: BrainStatus = IDLE;
  let timer: number | undefined;
  let open = false;

  const paintDot = () => {
    const dot = document.querySelector<HTMLElement>(".tab__dot");
    if (dot) dot.className = `tab__dot tab__dot--${status.dot}`;
  };

  const paint = () => {
    paintDot();
    if (!pane) return;
    const old = pane.querySelector<HTMLTextAreaElement>("textarea[name=chat]");
    const log = pane.querySelector<HTMLElement>(".chat__log");
    const draft = old?.value ?? "";
    const focused = !!old && document.activeElement === old;
    const atEnd = !log || log.scrollHeight - log.scrollTop - log.clientHeight < 40;
    const scroll = log?.scrollTop ?? 0;
    pane.replaceChildren(
      renderChat({ turns, status, draft }, {
        onSend: (text) => void invoke("chat_send", { text }).catch(() => undefined),
        onCancel: () => void invoke("window_cancel").catch(() => undefined),
        onStop: () => void invoke("brain_stop").catch(() => undefined),
      }),
    );
    const next = pane.querySelector<HTMLElement>(".chat__log");
    if (next) next.scrollTop = atEnd ? next.scrollHeight : scroll;
    if (focused) {
      const box = pane.querySelector<HTMLTextAreaElement>("textarea[name=chat]");
      box?.focus();
      box?.setSelectionRange(draft.length, draft.length);
    }
    // The countdown repaints each second while a batch waits.
    if (timer === undefined && status.pending?.until != null) {
      timer = window.setInterval(() => {
        if (!status.pending) {
          window.clearInterval(timer);
          timer = undefined;
          return;
        }
        paint();
      }, 1000);
    } else if (timer !== undefined && !status.pending) {
      window.clearInterval(timer);
      timer = undefined;
    }
  };

  const seen = () => void invoke("chat_seen").catch(() => undefined);

  void listen<ChatTurn>("chat", (e) => {
    turns = [...turns, e.payload];
    paint();
    if (open) seen();
  });
  void listen<BrainStatus>("brain", (e) => {
    status = e.payload;
    paint();
  });
  // makeTabs announces every switch; opening Chat means the user has read it.
  window.addEventListener("maya:tab", (ev) => {
    open = (ev as CustomEvent<string>).detail === "chat";
    if (open) seen();
  });
  void invoke<ChatTurn[]>("chat_history").then((t) => {
    turns = t;
    paint();
  }).catch(() => undefined);
  void invoke<BrainStatus>("brain_status").then((s) => {
    status = s;
    paint();
  }).catch(() => undefined);
  paint();
}
