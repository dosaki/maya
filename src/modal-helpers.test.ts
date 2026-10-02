import { describe, expect, it } from "vitest";
import { makeSendGuard, sameTurns } from "./modal";

describe("sameTurns", () => {
  it("compares turn lists by content", () => {
    expect(sameTurns([{ kind: "user", text: "a" }], [{ kind: "user", text: "a" }])).toBe(true);
    expect(sameTurns([{ kind: "user", text: "a" }], [{ kind: "peer", text: "a" }])).toBe(false);
    expect(sameTurns([], [{ kind: "user", text: "a" }])).toBe(false);
  });
});

describe("makeSendGuard", () => {
  it("ignores a second send while one is in flight", async () => {
    const calls: string[] = [];
    let release: () => void = () => {};
    const guarded = makeSendGuard(async (t: string) => {
      calls.push(t);
      await new Promise<void>((r) => {
        release = r;
      });
    });
    void guarded("one");
    void guarded("two");
    expect(calls).toEqual(["one"]);
    release();
    await new Promise((r) => setTimeout(r, 0));
    void guarded("three");
    expect(calls).toEqual(["one", "three"]);
  });

  it("guards sharing one lock block each other, so a reply and a command never run at once", async () => {
    const calls: string[] = [];
    let release: () => void = () => {};
    const lock = { inFlight: false };
    const wait = async (t: string) => {
      calls.push(t);
      await new Promise<void>((r) => {
        release = r;
      });
    };
    const reply = makeSendGuard(wait, lock);
    const command = makeSendGuard(wait, lock);
    void reply("hello");
    void command("/review");
    expect(calls).toEqual(["hello"]);
    release();
    await new Promise((r) => setTimeout(r, 0));
    void command("/review");
    expect(calls).toEqual(["hello", "/review"]);
  });
});
