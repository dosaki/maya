import { invoke } from "@tauri-apps/api/core";
import type { Card } from "./types";

export interface AnswerArgs {
  sessionId: string;
  /** The awaiting timestamp the buttons were rendered for; the backend refuses if the ask changed. */
  askId: number;
  questionIndex: number;
  optionIndex: number;
}

export type AnswerOutcome = "sent" | "dropped";

/**
 * Sends one answer per session at a time. The clicked button's whole option
 * group is disabled synchronously so a double click cannot type a second key
 * sequence into the terminal before the board repaints.
 */
export function makeAnswerGuard(send: (args: AnswerArgs) => Promise<void>) {
  const inFlight = new Set<string>();
  return {
    async answer(card: Card, questionIndex: number, optionIndex: number, button: HTMLElement): Promise<AnswerOutcome> {
      if (inFlight.has(card.sessionId)) return "dropped";
      inFlight.add(card.sessionId);
      const group = button.closest(".options") ?? button.parentElement;
      const buttons = [...(group?.querySelectorAll<HTMLButtonElement>("button[data-action=answer]") ?? [])];
      for (const b of buttons) b.disabled = true;
      try {
        await send({ sessionId: card.sessionId, askId: card.stateSince, questionIndex, optionIndex });
        return "sent";
      } catch (e) {
        for (const b of buttons) b.disabled = false;
        throw e;
      } finally {
        inFlight.delete(card.sessionId);
      }
    },
  };
}

/** The app-wide guard, backed by the Tauri command. */
export const answerGuard = makeAnswerGuard((args) => invoke("answer_question", { ...args }));
