import { beforeEach, describe, expect, it, vi } from "vitest";
import { makeCardHistory } from "./card-history";

describe("makeCardHistory", () => {
  beforeEach(() => {
    history.replaceState(null, "");
  });

  it("holds one entry per open card and pops it when the card is dismissed", () => {
    const back = vi.spyOn(history, "back").mockImplementation(() => undefined);
    const cards = makeCardHistory();
    const before = history.length;
    cards.opened("s1");
    expect(history.length).toBe(before + 1);
    expect(history.state).toEqual({ card: "s1" });
    cards.opened("s2");
    expect(history.length).toBe(before + 1);
    expect(history.state).toEqual({ card: "s2" });
    cards.dismissed();
    expect(back).toHaveBeenCalledTimes(1);
    back.mockRestore();
  });

  it("pops nothing when Back already took the card's entry", () => {
    const back = vi.spyOn(history, "back").mockImplementation(() => undefined);
    makeCardHistory().dismissed();
    expect(back).not.toHaveBeenCalled();
    back.mockRestore();
  });
});
