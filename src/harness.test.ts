import { describe, expect, it } from "vitest";
import { CAPABILITIES, capabilitiesOf } from "./harness";

describe("capabilities", () => {
  it("mirror the Rust table", () => {
    expect(CAPABILITIES["claude-code"]).toEqual({ compact: true, modelSwitch: true, effortSwitch: true, modeCycle: true, slashLines: true, shellLines: true });
    expect(CAPABILITIES.codex.modelSwitch).toBe(false);
    expect(CAPABILITIES.codex.compact).toBe(true);
    expect(CAPABILITIES.grok.modelSwitch).toBe(true);
    expect(capabilitiesOf("antigravity").modeCycle).toBe(true);
  });
});
