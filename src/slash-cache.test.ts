import { beforeEach, describe, expect, it, vi } from "vitest";
import { cachedCommands, commandsKey, rememberCommands } from "./slash-cache";
import type { CommandInfo } from "./types";

const cmd = (name: string): CommandInfo => ({ name, description: name, source: "builtin" });

describe("the commands cache", () => {
  beforeEach(() => localStorage.clear());

  it("remembers per machine, agent and folder", () => {
    const a = commandsKey({ machine: null, harness: "claude-code", cwd: "/x" });
    const b = commandsKey({ machine: "box", harness: "claude-code", cwd: "/x" });
    const c = commandsKey({ machine: null, harness: "codex", cwd: "/x" });
    const d = commandsKey({ machine: null, harness: "claude-code", cwd: "/y" });
    expect(new Set([a, b, c, d]).size).toBe(4);
    expect(a).toBe(["", "claude-code", "/x"].join("\u0000"));
    rememberCommands(a, [cmd("one")]);
    rememberCommands(b, [cmd("two")]);
    expect(cachedCommands(a)).toEqual([cmd("one")]);
    expect(cachedCommands(b)).toEqual([cmd("two")]);
    expect(cachedCommands(c)).toBeNull();
  });

  it("keeps the newest 40 lists", () => {
    for (let i = 0; i < 45; i++) rememberCommands(`k${i}`, [cmd(`c${i}`)]);
    expect(cachedCommands("k0")).toBeNull();
    expect(cachedCommands("k4")).toBeNull();
    expect(cachedCommands("k5")).toEqual([cmd("c5")]);
    expect(cachedCommands("k44")).toEqual([cmd("c44")]);
    rememberCommands("k5", [cmd("again")]);
    rememberCommands("new", [cmd("n")]);
    expect(cachedCommands("k5")).toEqual([cmd("again")]);
    expect(cachedCommands("k6")).toBeNull();
  });

  it("reads nothing, and throws nothing, when storage throws", () => {
    const get = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("no"); });
    const set = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("no"); });
    expect(cachedCommands("k")).toBeNull();
    expect(() => rememberCommands("k", [cmd("a")])).not.toThrow();
    get.mockRestore();
    set.mockRestore();
    localStorage.setItem("maya.slashCommands.v1", "not json");
    expect(cachedCommands("k")).toBeNull();
    expect(() => rememberCommands("k", [cmd("a")])).not.toThrow();
    expect(cachedCommands("k")).toEqual([cmd("a")]);
  });
});
