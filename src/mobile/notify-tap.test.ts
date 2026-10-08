import { beforeEach, describe, expect, it, vi } from "vitest";

const handlers: ((n: unknown) => void)[] = [];
vi.mock("@tauri-apps/plugin-notification", () => ({
  onAction: vi.fn(async (f: (n: unknown) => void) => {
    handlers.push(f);
  }),
}));

import { tappedSession, watchTaps } from "./notify-tap";

describe("tappedSession", () => {
  it("reads the session id from either payload shape the plugin has used", () => {
    expect(tappedSession({ id: 7, extra: { sessionId: "abc" } })).toBe("abc");
    expect(tappedSession({ actionId: "tap", notification: { id: 7, extra: { sessionId: "abc" } } })).toBe("abc");
    expect(tappedSession({ id: 7 })).toBeNull();
    expect(tappedSession({ extra: { sessionId: "" } })).toBeNull();
    expect(tappedSession(null)).toBeNull();
    expect(tappedSession("junk")).toBeNull();
  });
});

describe("watchTaps", () => {
  beforeEach(() => {
    handlers.length = 0;
  });

  it("opens the card of a notification tapped before the page listened, once, after listening", async () => {
    const order: string[] = [];
    const open = vi.fn((id: string) => order.push(`open ${id}`));
    const pending = vi.fn(async () => {
      order.push(`pending with ${handlers.length} listener`);
      return "s2";
    });
    await watchTaps(open, pending);
    expect(order).toEqual(["pending with 1 listener", "open s2"]);
    expect(pending).toHaveBeenCalledTimes(1);
  });

  it("opens nothing when no tap is pending", async () => {
    const open = vi.fn();
    await watchTaps(open, async () => null);
    expect(open).not.toHaveBeenCalled();
  });

  it("opens a warm tap's card and takes the copy kept for a cold start, without opening it twice", async () => {
    const open = vi.fn();
    const pending = vi.fn(async (): Promise<string | null> => null);
    await watchTaps(open, pending);
    pending.mockResolvedValueOnce("s1");
    handlers[0]({ notification: { extra: { sessionId: "s1" } } });
    await Promise.resolve();
    expect(open.mock.calls).toEqual([["s1"]]);
    expect(pending).toHaveBeenCalledTimes(2);
  });

  it("asks the app for the session when the action event does not name one (Android)", async () => {
    const open = vi.fn();
    const pending = vi.fn(async (): Promise<string | null> => null);
    await watchTaps(open, pending);
    pending.mockResolvedValueOnce("s3");
    handlers[0]({ actionId: "tap", inputValue: null, notification: null });
    await new Promise((r) => setTimeout(r, 0));
    expect(open.mock.calls).toEqual([["s3"]]);
  });

  it("stays quiet where there is no pending-tap command (a desktop run)", async () => {
    const open = vi.fn();
    await watchTaps(open, async () => {
      throw new Error("unknown command");
    });
    expect(open).not.toHaveBeenCalled();
  });
});
