import { describe, expect, it } from "vitest";
import { promoteLines } from "./promote";
import type { AssistantStatus } from "./settings";

const a = (id: string, name: string, more: Partial<AssistantStatus> = {}): AssistantStatus => ({ id, name, hostname: "h", platform: "macos", address: `10.0.0.${id.length}`, connected: true, lastSeen: null, canLead: true, canFollow: true, ...more });

describe("promoteLines", () => {
  it("says who will assist, who cannot follow and who is offline", () => {
    const list = [a("g", "laptop", { address: "192.168.55.70" }), a("bb", "mini"), a("ccc", "old", { canFollow: false, canLead: false }), a("dddd", "nas", { connected: false, canFollow: false, canLead: false })];
    expect(promoteLines(list, "g", "this Mac")).toEqual([
      "Make laptop the main Maya? Every connected Maya will assist laptop, and so will this Mac.",
      "old cannot follow and stays with no main; pair it with laptop afterwards.",
      "nas is offline; when it is back, pair it with laptop.",
    ]);
  });

  it("is only the headline when everyone follows", () => {
    expect(promoteLines([a("g", "laptop"), a("bb", "mini")], "g", "this phone")).toEqual(["Make laptop the main Maya? Every connected Maya will assist laptop, and so will this phone."]);
  });
});
