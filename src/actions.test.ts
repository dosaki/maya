import { describe, expect, it } from "vitest";
import { cardActionFor } from "./actions";
import { renderCard } from "./card";
import type { Card } from "./types";

const card: Card = { sessionId: "s1", pid: 9, name: "n", cwd: "/x/y", state: "idle", stateSince: 0, snippet: "", awaiting: null, hasInbox: true };

describe("cardActionFor", () => {
  it("routes the Terminal button to focus and everything else on the card to the modal", () => {
    const el = renderCard(card, 0);
    document.body.replaceChildren(el);
    expect(cardActionFor(el.querySelector("button[data-action=terminal]")!)).toEqual({ kind: "terminal", sessionId: "s1" });
    expect(cardActionFor(el.querySelector("button[data-action=reply]")!)).toEqual({ kind: "open", sessionId: "s1" });
    expect(cardActionFor(el.querySelector(".card__name")!)).toEqual({ kind: "open", sessionId: "s1" });
    expect(cardActionFor(document.body)).toBeNull();
  });

  it("routes option buttons to an answer action with indices", () => {
    const el = document.createElement("article");
    el.className = "card";
    el.dataset.sessionId = "s1";
    el.innerHTML = '<button data-action="answer" data-q="1" data-opt="2">x</button>';
    document.body.replaceChildren(el);
    expect(cardActionFor(el.querySelector("button")!)).toEqual({ kind: "answer", sessionId: "s1", questionIndex: 1, optionIndex: 2 });
  });
});
