import { invoke } from "@tauri-apps/api/core";
import { canStop } from "./harness";
import { OPEN_DELAY_MS } from "./options";
import type { Card, Pick } from "./types";

export interface PickArgs {
  sessionId: string;
  /** The awaiting timestamp the menu was rendered for; the backend refuses if the ask changed. */
  askId: number;
  questionIndex: number;
  pick: Pick;
}

export type AnswerOutcome = "sent" | "dropped";

/**
 * Sends one answer per session at a time. The clicked control's whole group
 * (the card's option buttons, or the menu's rows and own box) is disabled
 * synchronously so a double click cannot type a second key sequence into
 * the terminal before the board repaints.
 */
export function makeAnswerGuard(send: (args: PickArgs) => Promise<void>) {
  const inFlight = new Set<string>();
  const pick = async (card: Card, questionIndex: number, choice: Pick, el: HTMLElement): Promise<AnswerOutcome> => {
    if (inFlight.has(card.sessionId)) return "dropped";
    inFlight.add(card.sessionId);
    const group = el.closest(".options, .menu__rows") ?? el.parentElement;
    const controls = [...(group?.querySelectorAll<HTMLButtonElement | HTMLTextAreaElement>("button, textarea") ?? [])];
    for (const c of controls) c.disabled = true;
    group?.setAttribute("aria-disabled", "true");
    try {
      await send({ sessionId: card.sessionId, askId: card.stateSince, questionIndex, pick: choice });
      return "sent";
    } catch (e) {
      for (const c of controls) c.disabled = false;
      group?.removeAttribute("aria-disabled");
      throw e;
    } finally {
      inFlight.delete(card.sessionId);
    }
  };
  return {
    pick,
    answer: (card: Card, questionIndex: number, optionIndex: number, button: HTMLElement): Promise<AnswerOutcome> =>
      pick(card, questionIndex, { kind: "option", index: optionIndex }, button),
  };
}

/** The app-wide guard, backed by the Tauri command. */
export const answerGuard = makeAnswerGuard((args) => invoke("answer_pick", { ...args }));

/**
 * Sends Stop once per turn. Two Escs open Claude's rewind menu, so after a
 * click Stop stays disabled, across repaints, until the card's `stateSince`
 * moves; a failed stop gives it back.
 */
export function makeStopGuard(send: (sessionId: string) => Promise<void>) {
  /** The `stateSince` each session was stopped at. */
  const stoppedAt = new Map<string, number>();
  const stopping = (card: Card): boolean => stoppedAt.get(card.sessionId) === card.stateSince;
  return {
    /** True while a Stop for this turn was sent or is in flight. */
    stopping,
    async stop(card: Card, button: HTMLButtonElement): Promise<AnswerOutcome> {
      if (stopping(card)) return "dropped";
      stoppedAt.set(card.sessionId, card.stateSince);
      button.disabled = true;
      try {
        await send(card.sessionId);
        return "sent";
      } catch (e) {
        stoppedAt.delete(card.sessionId);
        button.disabled = false;
        throw e;
      }
    },
  };
}

/** The app-wide Stop guard, backed by the Tauri command. */
export const stopGuard = makeStopGuard((sessionId) => invoke("stop_session", { sessionId }));

/**
 * The Stop button for a card that has a turn to stop, or null. It is
 * disabled inside the open delay, so a tap meant for what was there before
 * cannot stop a turn that just started, and while a Stop is pending.
 */
export function stopButton(card: Card, nowMs: number, className = "card__btn"): HTMLButtonElement | null {
  if (!canStop(card)) return null;
  const b = document.createElement("button");
  b.type = "button";
  b.className = `${className} card__btn--stop`;
  b.dataset.action = "stop";
  b.textContent = "Stop";
  b.title = "Stop what the session is doing, as Esc does in its terminal";
  b.disabled = nowMs - card.stateSince < OPEN_DELAY_MS || stopGuard.stopping(card);
  return b;
}
