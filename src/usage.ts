import { contextBand, formatTokens, type AccountLimits, type Card, type Cost, type Harness, type LimitWindow, type SessionUsage } from "./types";

/** How much of the strip fits the session window's header, widest first. */
export type Stage = "wide" | "medium" | "narrow" | "chip";

/** The narrowest header, in pixels, each stage needs; below `narrow` is the chip. */
export const STAGE_MIN_PX = { wide: 1100, medium: 850, narrow: 650 } as const;

export function stageFor(headWidthPx: number): Stage {
  if (headWidthPx >= STAGE_MIN_PX.wide) return "wide";
  if (headWidthPx >= STAGE_MIN_PX.medium) return "medium";
  if (headWidthPx >= STAGE_MIN_PX.narrow) return "narrow";
  return "chip";
}

/** "$4.12" (two decimals, micros rounded), "12.4 credits" (one decimal), with " est." appended when estimated. */
export function costText(cost: Cost): string {
  const amount = cost.unit === "usd" ? `$${(cost.micros / 1_000_000).toFixed(2)}` : `${(cost.micros / 1_000_000).toFixed(1)} credits`;
  return cost.estimated ? `${amount} est.` : amount;
}

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const DAY_MS = 24 * 60 * 60 * 1000;
const pad = (n: number) => String(n).padStart(2, "0");

/** "1h 12m" or "45m" until a reset within a day, "Thu 09:00" for one further off; null when unknown or past. */
function resetWhen(resetsAt: number | null | undefined, nowMs: number): { text: string; far: boolean } | null {
  if (resetsAt === null || resetsAt === undefined) return null;
  const at = resetsAt * 1000;
  const left = at - nowMs;
  if (left <= 0) return null;
  if (left > DAY_MS) {
    const d = new Date(at);
    return { text: `${WEEKDAYS[d.getDay()]} ${pad(d.getHours())}:${pad(d.getMinutes())}`, far: true };
  }
  const minutes = Math.ceil(left / 60_000);
  const h = Math.floor(minutes / 60);
  return { text: h > 0 ? `${h}h ${minutes % 60}m` : `${minutes}m`, far: false };
}

/** "↻ 1h 12m" / "↻ 45m" / "↻ Thu 09:00" when more than a day away; "" when unknown. */
export function resetText(resetsAt: number | null | undefined, nowMs: number): string {
  const when = resetWhen(resetsAt, nowMs);
  return when ? `↻ ${when.text}` : "";
}

/** The agents whose account limits Maya reads. */
const READS_LIMITS: Harness[] = ["claude-code", "codex", "antigravity", "kiro"];

/** The muted note when there are no windows: Grok → "limits not shared"; OpenCode → "no limits"; limits present but empty → "no limits"; Claude Code / Codex / Antigravity / Kiro with limits null → "limits not read yet". */
export function limitsNote(harness: Harness, limits: AccountLimits | null | undefined): string | null {
  if (harness === "grok") return "limits not shared";
  if (harness === "opencode") return "no limits";
  if (limits) return limits.windows.length === 0 ? "no limits" : null;
  return READS_LIMITS.includes(harness) ? "limits not read yet" : null;
}

function windowWords(label: string): string {
  if (label === "5h") return "5-hour limit";
  if (label === "7d") return "Weekly limit";
  return `${label} limit`;
}

/** A reading older than this says how old it is. */
const STALE_READ_MS = 5 * 60_000;

/** "12m", "3h", "2d": how long ago a reading was taken. */
function age(ms: number): string {
  const m = Math.floor(ms / 60_000);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  return h < 24 ? `${h}h` : `${Math.floor(h / 24)}d`;
}

/**
 * Tooltip of a window: "5-hour limit 78% · resets in 1h 12m · tco@example.com"
 * (label words: 5h → "5-hour limit", 7d → "Weekly limit", else "<label> limit"),
 * then " · read 12m ago" when `readAt` (epoch ms) is more than 5 minutes old.
 */
export function windowTitle(w: LimitWindow, account: string | null | undefined, nowMs: number, readAt?: number | null): string {
  const when = resetWhen(w.resetsAt, nowMs);
  const reset = when ? (when.far ? `resets ${when.text}` : `resets in ${when.text}`) : null;
  const stale = readAt !== null && readAt !== undefined && nowMs - readAt > STALE_READ_MS ? `read ${age(nowMs - readAt)} ago` : null;
  return [`${windowWords(w.label)} ${w.percent}%`, reset, account || null, stale].filter(Boolean).join(" · ");
}

/** "512", "12k", "1.2M": a token count for the strip. */
function tokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return formatTokens(n);
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
}

function totalTokens(u: SessionUsage): number {
  return u.input + u.cachedInput + u.cacheWrite + u.output;
}

/** Tooltip of the cost: tokens by kind, then "Estimated at list prices for <models>" or "Reported by the agent", or "main conversation only" for an estimated Claude Code cost. */
export function costTitle(usage: SessionUsage, harness: Harness): string {
  const lines: string[] = [];
  if (totalTokens(usage) > 0) {
    lines.push(`${tokens(usage.input)} in · ${tokens(usage.cachedInput)} cached · ${tokens(usage.cacheWrite)} cache write · ${tokens(usage.output)} out`);
  }
  const cost = usage.cost;
  if (cost?.estimated) {
    const est = cost.models.length ? `Estimated at list prices for ${cost.models.join(", ")}` : "Estimated at list prices";
    lines.push(harness === "claude-code" ? `${est} · main conversation only` : est);
  } else if (cost) {
    lines.push("Reported by the agent");
  }
  return lines.join("\n");
}

function hottestWindow(limits: AccountLimits | null | undefined): number | null {
  if (!limits || limits.windows.length === 0) return null;
  return Math.max(...limits.windows.map((w) => w.percent));
}

/** The band of the hottest window and the context, for the chip's colour. */
export function hottestBand(card: Card): "low" | "mid" | "high" {
  const percents = [card.context?.percent, hottestWindow(card.limits)].filter((p): p is number => typeof p === "number");
  return percents.length ? contextBand(Math.max(...percents)) : "low";
}

function span(className: string, text?: string): HTMLSpanElement {
  const s = document.createElement("span");
  s.className = className;
  if (text !== undefined) s.textContent = text;
  return s;
}

const clamp = (p: number) => Math.min(100, Math.max(0, p));

interface MeterPart {
  kind: "ctx" | "limit";
  label: string;
  percent: number;
  reset: string;
  title: string;
}

function meters(card: Card, nowMs: number): MeterPart[] {
  const parts: MeterPart[] = [];
  const ctx = card.context;
  if (ctx) {
    // A recording may carry no window size: the percent alone, no division.
    const title = ctx.window > 0 ? `Context ${ctx.percent}% · ${formatTokens(ctx.used)} of ${formatTokens(ctx.window)}` : `Context ${ctx.percent}%`;
    parts.push({ kind: "ctx", label: "ctx", percent: ctx.percent, reset: "", title });
  }
  for (const w of card.limits?.windows ?? []) {
    parts.push({ kind: "limit", label: w.label, percent: w.percent, reset: resetText(w.resetsAt, nowMs), title: windowTitle(w, card.limits?.account, nowMs, card.limits?.readAt) });
  }
  return parts;
}

/** The strip element: class `usage usage--<stage>`, children `.usage__meter[data-kind=ctx|limit]` each with `.usage__label`, `.usage__bar > .usage__fill`, `.usage__pct`, `.usage__reset`; `.usage__cost` with `.usage__tokens`; `.usage__note` for the note; at the chip stage a single `.usage__chip` with `data-band`. */
export function usageStrip(card: Card, stage: Stage, nowMs: number = Date.now()): HTMLElement {
  const strip = span(`usage usage--${stage}`);
  strip.dataset.stage = stage;
  const parts = meters(card, nowMs);
  const usage = card.usage ?? null;
  const cost = usage?.cost ? costText(usage.cost) : null;
  const count = usage && totalTokens(usage) > 0 ? `${tokens(totalTokens(usage))} tok` : null;
  const note = limitsNote(card.harness, card.limits);

  if (stage === "chip") {
    const ctxPart = card.context ? `${card.context.percent}%` : null;
    const hottest = hottestWindow(card.limits);
    const chip = span("usage__chip", [ctxPart, hottest === null ? null : `${hottest}%`, cost].filter(Boolean).join(" · "));
    chip.dataset.band = hottestBand(card);
    chip.title = [
      ...parts.map((p) => [`${p.label} ${p.percent}%`, p.reset].filter(Boolean).join(" ")),
      [cost, count].filter(Boolean).join(" ") || null,
      note,
    ]
      .filter(Boolean)
      .join(" · ");
    strip.append(chip);
    return strip;
  }

  const detailed = stage === "wide";
  for (const p of parts) {
    const meter = span("usage__meter");
    meter.dataset.kind = p.kind;
    meter.dataset.band = contextBand(p.percent);
    meter.title = p.title;
    meter.append(span("usage__label", p.label));
    if (stage !== "narrow") {
      const bar = span("usage__bar");
      const fill = span("usage__fill");
      fill.style.width = `${clamp(p.percent)}%`;
      bar.append(fill);
      meter.append(bar);
    }
    meter.append(span("usage__pct", `${p.percent}%`));
    if (detailed && p.reset) meter.append(span("usage__reset", p.reset));
    strip.append(meter);
  }
  if (usage && (cost || count)) {
    const box = span("usage__cost");
    box.title = costTitle(usage, card.harness);
    if (cost) box.append(span("usage__amount", cost));
    if (detailed && count) box.append(span("usage__tokens", count));
    if (box.childElementCount > 0) strip.append(box);
  }
  if (note) strip.append(span("usage__note", note));
  return strip;
}
