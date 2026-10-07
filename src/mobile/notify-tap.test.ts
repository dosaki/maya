import { describe, expect, it } from "vitest";
import { tappedSession } from "./notify-tap";

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
