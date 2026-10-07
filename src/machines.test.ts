import { afterEach, describe, expect, it } from "vitest";
import { hasLocalMachine, initialChoices, machineChoices, setLocalMachine } from "./machines";

const laptop = { name: "laptop", hostname: "laptop", platform: "macos", connected: true };
const away = { name: "mini", hostname: "mini", platform: "linux", connected: false };

describe("machine choices", () => {
  afterEach(() => setLocalMachine(true));

  it("lists this machine first, then connected assistants only", () => {
    expect(hasLocalMachine()).toBe(true);
    expect(machineChoices([laptop, away]).map((m) => m.value)).toEqual(["", "laptop"]);
    expect(machineChoices([laptop])[0].name).toMatch(/^This /);
    expect(initialChoices()).toHaveLength(1);
  });

  it("without a local machine lists assistants alone", () => {
    setLocalMachine(false);
    expect(hasLocalMachine()).toBe(false);
    expect(machineChoices([laptop, away])).toEqual([{ name: "laptop", value: "laptop" }]);
    expect(machineChoices([away])).toEqual([]);
    expect(initialChoices()).toEqual([]);
  });
});
