import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => undefined) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => undefined) }));

import { gitConnections, renderSettings, workItemsConfig, type SettingsModel, type NetworkStatus } from "./settings";
import { PHONE_LIST, renderNetwork, type NetworkModel, type PhoneGroup } from "./mobile/network";

// Every control a screen draws has a name a screen reader can read: a wrapping
// <label>, an aria-label, or an aria-labelledby that points at text on the page.

const CONTROLS = "input, select, textarea, [role=radiogroup]";

function nameOf(c: Element, root: Element): string {
  const label = c.getAttribute("aria-label")?.trim();
  if (label) return label;
  const by = c.getAttribute("aria-labelledby");
  if (by) {
    return by.split(/\s+/).map((id) => root.querySelector(`[id="${id}"]`)?.textContent?.trim() ?? "").join(" ").trim();
  }
  if (c.id) {
    const forLabel = root.querySelector(`label[for="${c.id}"]`);
    if (forLabel?.textContent?.trim()) return forLabel.textContent.trim();
  }
  if (c.matches("input, select, textarea")) {
    const wrapping = c.closest("label");
    if (wrapping) {
      return wrapping.textContent?.trim() ?? "";
    }
  }
  return "";
}

function unnamed(root: Element): string[] {
  return [...root.querySelectorAll(CONTROLS)]
    .filter((c) => !(c instanceof HTMLInputElement && c.type === "hidden"))
    .filter((c) => nameOf(c, root) === "")
    .map((c) => `${c.tagName.toLowerCase()}${(c as HTMLInputElement).name ? `[name=${(c as HTMLInputElement).name}]` : ""}${c.getAttribute("role") ? `[role=${c.getAttribute("role")}]` : ""}`);
}

const rule = (over: { labels?: string[]; repos?: string[] } = {}) => ({ assignee: { kind: "me" as const }, labels: [] as string[], repos: [] as string[], ...over });
const forge = { enabled: true, rules: [rule(), rule({ labels: ["bug"] })], statusLabels: ["todo", "in progress"], startStatus: "in progress", trackerRepos: [] };
const tool = { id: "custom-1", name: "Linear", enabled: true, listCommand: "~/bin/linear-items.sh", startCommand: "", me: "" };

const devices = [
  { id: "me", key: "pk-me", name: "Studio", hostname: "studio", platform: "macos", address: "10.0.0.1", lastSeen: null, isMain: true, isSelf: true },
  { id: "p", key: "pk-p", name: "Pixel", hostname: "pixel", platform: "android", address: "10.0.0.5", lastSeen: 1, isMain: false, isSelf: false },
];
const mainNet: NetworkStatus = {
  role: "main",
  code: { code: "123456", expiresAt: 9e12 },
  assistants: [{ id: "a1", name: "Pixel", platform: "android", address: "10.0.0.5", connected: true, lastSeen: 1, note: null, canLead: true, mirror: false, wantsMirror: true }],
  assistant: { connected: false, mainName: null, error: null },
  devices,
} as unknown as NetworkStatus;
const memberNet: NetworkStatus = {
  role: "assistant",
  code: null,
  assistants: [],
  assistant: { connected: true, mainName: "Studio", error: null },
  devices: [
    { id: "me", key: "pk-me", name: "Laptop", hostname: "laptop", platform: "linux", address: "", lastSeen: null, isMain: false, isSelf: true },
    { id: "m", key: "pk-main", name: "Studio", hostname: "studio", platform: "macos", address: "10.0.0.1", lastSeen: 1, isMain: true, isSelf: false },
    { id: "d", key: "pk-desk", name: "Desk", hostname: "desk", platform: "windows", address: "10.0.0.3", lastSeen: 1, isMain: false, isSelf: false },
  ],
} as unknown as NetworkStatus;

const base: SettingsModel = {
  hookInstalled: true,
  completedTimeoutMinutes: 30,
  projectsDir: "",
  clonesDir: "",
  notifyOnAwaiting: true,
  speakNotifications: true,
  elevenEnabled: true,
  elevenKeySet: true,
  elevenVoices: [{ voiceId: "v1", name: "Rachel" }],
  elevenVoiceId: "v1",
  serverEnabled: true,
  serverUrl: "http://localhost:8000",
  serverVoice: "a",
  serverModel: "m",
  serverKeySet: false,
  serverVoices: ["a", "b"],
  serverVoicesError: null,
  voiceReport: null,
  error: null,
  listen: true,
  microphone: "",
  microphones: ["Built-in"],
  agent: "claude-code",
  agentModel: "",
  agents: [{ harness: "claude-code", models: [{ id: "opus", label: "Opus", efforts: [] }], efforts: [], modes: [] }] as SettingsModel["agents"],
  reviewPrompt: "",
  listenError: null,
  recognizer: "builtin",
  whisperModel: "base.en-q5_1",
  models: [{ id: "base.en-q5_1", name: "Base", size: 1, installed: true }] as unknown as SettingsModel["models"],
  downloading: null,
  git: gitConnections([
    { provider: "github", enabled: true, token: { kind: "stored" } },
    { provider: "gitlab", enabled: true, token: { kind: "env", name: "GITLAB_TOKEN" } },
  ]),
  gitStatus: { github: { hasToken: true }, gitlab: { hasToken: false } },
  workItems: { github: forge, gitlab: forge, coral: workItemsConfig(null).coral, prompt: "", custom: [tool] },
  sourcesOpen: { github: true, gitlab: true, "custom-1": true },
  network: mainNet,
  networkRole: "main",
  networkPort: 4127,
  networkMainHost: "",
  networkMainPort: 0,
  networkName: "Studio",
};

const noop = () => undefined;
const handlers = new Proxy({}, { get: () => noop }) as Parameters<typeof renderSettings>[1];

describe("accessible names", () => {
  it("names every control on every desktop screen", () => {
    const screens: [string, SettingsModel][] = [
      ["main", base],
      ["main with a Join form", { ...base, network: { ...memberNet, role: "assistant" }, networkRole: "assistant", devicesJoining: "pk-desk" } as SettingsModel],
      ["Whisper", { ...base, recognizer: "builtin", elevenEnabled: false, serverEnabled: false }],
      ["system recognizer", { ...base, recognizer: "system", git: gitConnections(null) }],
    ];
    for (const [label, model] of screens) {
      const root = renderSettings(model, handlers);
      expect({ label, unnamed: unnamed(root) }).toEqual({ label, unnamed: [] });
    }
  });

  it("names every control on every phone page", () => {
    const handlersPhone = new Proxy({ devices: new Proxy({}, { get: () => noop }) }, { get: (t, k) => (k in t ? (t as never)[k] : noop) }) as unknown as Parameters<typeof renderNetwork>[1];
    const phone: NetworkModel = { mainHost: "", mainPort: 0, status: memberNet, name: "Pixel", notifyAwaiting: true, notifyCompleted: false, speak: true, wakeWord: false, notificationsAllowed: true, busy: false, error: null, devicesJoining: "pk-desk", newKey: true };
    const groups: PhoneGroup[] = PHONE_LIST.flat();
    const offStatus = { ...memberNet, role: "off", assistant: { connected: false, mainName: null, error: null } } as unknown as NetworkStatus;
    for (const status of [memberNet, offStatus]) {
      for (const group of groups) {
        const root = renderNetwork({ ...phone, status, group }, handlersPhone);
        expect({ group, unnamed: unnamed(root) }).toEqual({ group, unnamed: [] });
      }
      const list = renderNetwork({ ...phone, status }, handlersPhone);
      expect(unnamed(list)).toEqual([]);
    }
  });
});
