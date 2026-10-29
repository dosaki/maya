import { describe, expect, it } from "vitest";
import type { AccountLimits, Card, SessionUsage } from "./types";
import { costText, costTitle, hottestBand, limitsNote, resetText, stageFor, usageStrip, windowTitle } from "./usage";

const NOW = Date.UTC(2026, 9, 29, 9, 0, 0);
const MIN = 60_000;
const secs = (ms: number) => Math.round(ms / 1000);

const base: Card = { sessionId: "s", pid: 1, name: "eye-1", cwd: "/x/dev/eye", state: "idle", stateSince: 0, snippet: "", awaiting: null, hasInbox: true, harness: "claude-code", pr: null, context: null };
const limits: AccountLimits = {
  account: "tco@example.com",
  windows: [
    { label: "5h", percent: 78, resetsAt: secs(NOW + 72 * MIN) },
    { label: "7d", percent: 41, resetsAt: secs(NOW + 3 * 24 * 60 * MIN) },
  ],
  readAt: NOW,
};
const usage: SessionUsage = { input: 12_000, cachedInput: 1_200_000, cacheWrite: 30_000, output: 8_000, cost: { micros: 4_123_400, unit: "usd", estimated: true, models: ["claude-opus-5"] } };
const claude: Card = { ...base, context: { used: 124_000, window: 200_000, percent: 62 }, limits, usage };

describe("the usage strip's helpers", () => {
  it("stages_follow_the_thresholds", () => {
    expect(stageFor(1100)).toBe("wide");
    expect(stageFor(1099)).toBe("medium");
    expect(stageFor(850)).toBe("medium");
    expect(stageFor(849)).toBe("narrow");
    expect(stageFor(650)).toBe("narrow");
    expect(stageFor(649)).toBe("chip");
  });

  it("cost_text_rounds_dollars_and_credits_and_marks_estimates", () => {
    expect(costText({ micros: 4_123_400, unit: "usd", estimated: true, models: [] })).toBe("$4.12 est.");
    expect(costText({ micros: 12_400_000, unit: "credits", estimated: false, models: [] })).toBe("12.4 credits");
    expect(costText({ micros: 4_125_000, unit: "usd", estimated: false, models: [] })).toBe("$4.13");
    // A free OpenCode model costs nothing, which is known.
    expect(costText({ micros: 0, unit: "usd", estimated: false, models: [] })).toBe("$0.00");
  });

  it("reset_text_counts_down_or_names_the_day", () => {
    expect(resetText(secs(NOW + 72 * MIN), NOW)).toBe("↻ 1h 12m");
    expect(resetText(secs(NOW + 45 * MIN), NOW)).toBe("↻ 45m");
    const far = resetText(secs(NOW + 3 * 24 * 60 * MIN), NOW);
    expect(far.startsWith("↻ ")).toBe(true);
    expect(far).toMatch(/\b(Mon|Tue|Wed|Thu|Fri|Sat|Sun) \d\d:\d\d$/);
    expect(resetText(null, NOW)).toBe("");
    expect(resetText(undefined, NOW)).toBe("");
  });

  it("limits_note_per_harness", () => {
    expect(limitsNote("grok", null)).toBe("limits not shared");
    expect(limitsNote("opencode", null)).toBe("no limits");
    expect(limitsNote("claude-code", { account: null, windows: [], readAt: NOW })).toBe("no limits");
    for (const h of ["claude-code", "codex", "antigravity", "kiro"] as const) {
      expect(limitsNote(h, null), h).toBe("limits not read yet");
      expect(limitsNote(h, undefined), h).toBe("limits not read yet");
    }
    expect(limitsNote("claude-code", limits)).toBeNull();
  });

  it("names the window, its reset and the account in its tooltip, and the age of an old reading", () => {
    expect(windowTitle(limits.windows[0], "tco@example.com", NOW)).toBe("5-hour limit 78% · resets in 1h 12m · tco@example.com");
    expect(windowTitle(limits.windows[1], null, NOW)).toMatch(/^Weekly limit 41% · resets (Mon|Tue|Wed|Thu|Fri|Sat|Sun) \d\d:\d\d$/);
    expect(windowTitle({ label: "month", percent: 30, resetsAt: null }, null, NOW)).toBe("month limit 30%");
    expect(windowTitle({ label: "5h", percent: 5, resetsAt: null }, null, NOW, NOW - 4 * MIN)).toBe("5-hour limit 5%");
    expect(windowTitle({ label: "5h", percent: 5, resetsAt: null }, null, NOW, NOW - 12 * MIN)).toBe("5-hour limit 5% · read 12m ago");
    expect(windowTitle({ label: "5h", percent: 5, resetsAt: null }, null, NOW, NOW - 3 * 60 * MIN)).toBe("5-hour limit 5% · read 3h ago");
  });

  it("explains the cost: tokens by kind, then where the figure came from", () => {
    const estimated = costTitle(usage, "claude-code");
    expect(estimated).toContain("12k in");
    expect(estimated).toContain("1.2M cached");
    expect(estimated).toContain("8k out");
    expect(estimated).toContain("Estimated at list prices for claude-opus-5");
    expect(estimated).toContain("main conversation only");
    const reported = costTitle({ ...usage, cost: { micros: 4_000_000, unit: "usd", estimated: false, models: [] } }, "claude-code");
    expect(reported).toContain("Reported by the agent");
    expect(reported).not.toContain("main conversation only");
    expect(costTitle({ ...usage, cost: { ...usage.cost!, models: ["gpt-5.5"] } }, "codex")).not.toContain("main conversation only");
    // Kiro's credits come with no tokens.
    const kiro = costTitle({ input: 0, cachedInput: 0, cacheWrite: 0, output: 0, cost: { micros: 12_400_000, unit: "credits", estimated: false, models: [] } }, "kiro");
    expect(kiro).toBe("Reported by the agent");
  });

  it("colours the chip by the hottest of the context and the windows", () => {
    expect(hottestBand(claude)).toBe("high");
    expect(hottestBand({ ...base, context: { used: 1, window: 2, percent: 55 } })).toBe("mid");
    expect(hottestBand(base)).toBe("low");
  });
});

describe("usageStrip", () => {
  it("the_strip_drops_detail_by_stage", () => {
    const wide = usageStrip(claude, "wide", NOW);
    expect(wide.className).toBe("usage usage--wide");
    expect(wide.dataset.stage).toBe("wide");
    expect(wide.querySelectorAll(".usage__meter")).toHaveLength(3);
    expect(wide.querySelectorAll(".usage__meter[data-kind=ctx]")).toHaveLength(1);
    expect(wide.querySelectorAll(".usage__meter[data-kind=limit]")).toHaveLength(2);
    expect(wide.querySelectorAll(".usage__reset")).toHaveLength(2);
    expect(wide.querySelectorAll(".usage__bar > .usage__fill")).toHaveLength(3);
    expect(wide.querySelector(".usage__tokens")).not.toBeNull();
    expect(wide.querySelector(".usage__cost")?.textContent).toContain("$4.12 est.");
    expect(wide.querySelector(".usage__meter[data-kind=ctx] .usage__pct")?.textContent).toBe("62%");
    expect(wide.querySelector<HTMLElement>(".usage__meter[data-kind=ctx]")?.title).toBe("Context 62% · 124k of 200k");
    expect(wide.querySelector<HTMLElement>(".usage__meter[data-kind=limit]")?.title).toBe("5-hour limit 78% · resets in 1h 12m · tco@example.com");
    expect(wide.querySelector(".usage__note")).toBeNull();

    const medium = usageStrip(claude, "medium", NOW);
    expect(medium.querySelectorAll(".usage__meter")).toHaveLength(3);
    expect(medium.querySelector(".usage__reset")).toBeNull();
    expect(medium.querySelector(".usage__tokens")).toBeNull();
    expect(medium.querySelector(".usage__bar")).not.toBeNull();

    const narrow = usageStrip(claude, "narrow", NOW);
    expect(narrow.querySelectorAll(".usage__meter")).toHaveLength(3);
    expect(narrow.querySelector(".usage__bar")).toBeNull();
    expect(narrow.querySelector(".usage__cost")).not.toBeNull();

    const chip = usageStrip(claude, "chip", NOW);
    const chips = chip.querySelectorAll<HTMLElement>(".usage__chip");
    expect(chips).toHaveLength(1);
    expect(chip.children).toHaveLength(1);
    expect(chips[0].textContent).toBe("62% · 78% · $4.12 est.");
    expect(chips[0].dataset.band).toBe("high");
    expect(chips[0].title).toContain("ctx 62%");
    expect(chips[0].title).toContain("5h 78%");
    expect(chips[0].title).toContain("7d 41%");
    expect(chips[0].title).toContain("$4.12 est.");
  });

  it("a_bare_card_at_the_chip_stage_shows_only_context", () => {
    const grok: Card = { ...base, harness: "grok", context: { used: 16_000, window: 200_000, percent: 8 } };
    const chip = usageStrip(grok, "chip", NOW).querySelector<HTMLElement>(".usage__chip")!;
    expect(chip.textContent).toBe("8%");
    expect(chip.dataset.band).toBe("low");
    expect(usageStrip(grok, "wide", NOW).querySelector(".usage__note")?.textContent).toBe("limits not shared");
    expect(() => usageStrip({ ...base, harness: "other" }, "chip", NOW)).not.toThrow();
  });

  it("antigravity_and_kiro_windows_keep_their_labels", () => {
    const agy: Card = { ...base, harness: "antigravity", limits: { account: null, windows: [{ label: "Gemini 7d", percent: 12, resetsAt: null }, { label: "Claude+GPT 7d", percent: 40, resetsAt: null }], readAt: NOW } };
    expect([...usageStrip(agy, "wide", NOW).querySelectorAll(".usage__meter[data-kind=limit] .usage__label")].map((l) => l.textContent)).toEqual(["Gemini 7d", "Claude+GPT 7d"]);
    const kiro: Card = { ...base, harness: "kiro", limits: { account: null, windows: [{ label: "month", percent: 30, resetsAt: null }], readAt: NOW }, usage: { input: 0, cachedInput: 0, cacheWrite: 0, output: 0, cost: { micros: 12_400_000, unit: "credits", estimated: false, models: [] } } };
    const strip = usageStrip(kiro, "wide", NOW);
    expect(strip.querySelector(".usage__meter[data-kind=limit] .usage__label")?.textContent).toBe("month");
    expect(strip.querySelector(".usage__cost")?.textContent).toBe("12.4 credits");
    expect(strip.querySelector(".usage__tokens"), "no tokens to count").toBeNull();
    expect(strip.querySelector(".usage__reset"), "no reset known").toBeNull();
  });

  it("shows only the percent when the context window is unknown", () => {
    const strip = usageStrip({ ...base, context: { used: 0, window: 0, percent: 40 } }, "wide", NOW);
    expect(strip.querySelector<HTMLElement>(".usage__meter[data-kind=ctx]")?.title).toBe("Context 40%");
    expect(strip.querySelector(".usage__meter[data-kind=ctx] .usage__pct")?.textContent).toBe("40%");
  });

  it("shows a free model's cost as $0.00 and a missing reading as a note", () => {
    const opencode: Card = { ...base, harness: "opencode", usage: { input: 500, cachedInput: 0, cacheWrite: 0, output: 200, cost: { micros: 0, unit: "usd", estimated: false, models: [] } } };
    const strip = usageStrip(opencode, "wide", NOW);
    expect(strip.querySelector(".usage__cost")?.textContent).toContain("$0.00");
    expect(strip.querySelector(".usage__note")?.textContent).toBe("no limits");
    expect(usageStrip(base, "wide", NOW).querySelector(".usage__note")?.textContent).toBe("limits not read yet");
  });
});
