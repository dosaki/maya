import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { KEYS_NOTICE_KEY, maybeShowKeysNotice } from "./notice";

const notice = () => document.querySelector<HTMLElement>(".notice");

beforeEach(() => {
  document.body.innerHTML = "";
  localStorage.clear();
  invoke.mockReset();
  vi.restoreAllMocks();
});

const withConfig = (config: object) => invoke.mockImplementation((cmd: string) => (cmd === "get_config" ? Promise.resolve(config) : Promise.reject(new Error("unexpected " + cmd))));

describe("the new keys notice", () => {
  it("tells someone who updated that Enter now sends, and where to choose their own keys", async () => {
    withConfig({ agent: "claude-code" });
    await maybeShowKeysNotice(() => {});
    const text = notice()?.textContent ?? "";
    expect(text).toContain("Enter now sends");
    expect(text).toContain("Settings › Keyboard");
  });

  it("shows once: a dismissed notice stays away on the next start", async () => {
    withConfig({ agent: "claude-code" });
    await maybeShowKeysNotice(() => {});
    notice()!.querySelector<HTMLButtonElement>("button[data-action=notice-dismiss]")!.click();
    expect(notice()).toBeNull();
    await maybeShowKeysNotice(() => {});
    expect(notice()).toBeNull();
  });

  it("opens Settings › Keyboard and goes away", async () => {
    withConfig({ agent: "claude-code" });
    const openKeyboard = vi.fn();
    await maybeShowKeysNotice(openKeyboard);
    expect(openKeyboard).not.toHaveBeenCalled();
    notice()!.querySelector<HTMLButtonElement>("button[data-action=notice-open]")!.click();
    expect(openKeyboard).toHaveBeenCalledTimes(1);
    expect(notice()).toBeNull();
    await maybeShowKeysNotice(openKeyboard);
    expect(notice()).toBeNull();
  });

  it("says nothing on a fresh install, then or later", async () => {
    withConfig({ agent: null });
    await maybeShowKeysNotice(() => {});
    expect(notice()).toBeNull();
    withConfig({ agent: "claude-code" });
    await maybeShowKeysNotice(() => {});
    expect(notice()).toBeNull();
  });

  it("says nothing to someone who has already chosen their keys", async () => {
    withConfig({ agent: "claude-code", keys: { send: ["Meta+Enter"], newLine: [] } });
    await maybeShowKeysNotice(() => {});
    expect(notice()).toBeNull();
  });

  it("says nothing when it could not remember being shown, rather than every start", async () => {
    withConfig({ agent: "claude-code" });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("storage is off");
    });
    await maybeShowKeysNotice(() => {});
    expect(notice()).toBeNull();
    expect(localStorage.getItem(KEYS_NOTICE_KEY)).toBeNull();
  });

  it("says nothing when the config cannot be read", async () => {
    invoke.mockRejectedValue(new Error("no config"));
    await maybeShowKeysNotice(() => {});
    expect(notice()).toBeNull();
  });
});
