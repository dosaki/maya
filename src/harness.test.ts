import { describe, expect, it } from "vitest";
import { CAPABILITIES, capabilitiesOf } from "./harness";
import type { Harness } from "./types";

describe("capabilities", () => {
  it("mirror the Rust table", () => {
    expect(CAPABILITIES).toEqual({
      "claude-code": { compact: true, modelSwitch: true, effortSwitch: true, modeCycle: true, slashLines: true, shellLines: true },
      codex: { compact: true, modelSwitch: false, effortSwitch: false, modeCycle: false, slashLines: true, shellLines: false },
      antigravity: { compact: true, modelSwitch: false, effortSwitch: false, modeCycle: true, slashLines: true, shellLines: false },
      grok: { compact: true, modelSwitch: true, effortSwitch: false, modeCycle: true, slashLines: true, shellLines: false },
    });
    expect(capabilitiesOf("antigravity")).toBe(CAPABILITIES.antigravity);
  });

  it("offer nothing for an agent this build does not know", () => {
    expect(capabilitiesOf("opencode" as Harness)).toEqual({ compact: false, modelSwitch: false, effortSwitch: false, modeCycle: false, slashLines: false, shellLines: false });
  });
});
