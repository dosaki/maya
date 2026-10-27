import { describe, expect, it } from "vitest";
import { chatTabShown } from "./chat-tab";
import type { NetworkStatus } from "./network";

const joined = (mirror?: "allowed" | "refused" | "unsupported" | null): NetworkStatus => ({
  role: "assistant",
  code: null,
  assistants: [],
  assistant: { connected: true, mainName: "desk", error: null, mirror },
  mainError: null,
});

describe("chatTabShown", () => {
  it("shows only for a joined phone whose main allows it", () => {
    expect(chatTabShown(joined("allowed"))).toBe(true);
    expect(chatTabShown(joined("refused"))).toBe(false);
    expect(chatTabShown(joined("unsupported"))).toBe(false);
    expect(chatTabShown(joined(null))).toBe(false);
    expect(chatTabShown({ ...joined("allowed"), role: "off" })).toBe(false);
  });
});
