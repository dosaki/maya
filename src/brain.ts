import { invoke } from "@tauri-apps/api/core";
import type { AgentInfo } from "./newsession";
import type { Harness } from "./types";

/** The agent that powers Maya, from the config; Claude Code when unset or unreadable. */
export async function brainAgent(): Promise<Harness> {
  try {
    const c = await invoke<{ agent?: Harness | null }>("get_config");
    return c.agent ?? "claude-code";
  } catch {
    return "claude-code";
  }
}

/** The agent a modal starts on: Maya's when the machine has it, else the first listed. */
export function defaultAgent(agents: AgentInfo[], brain: Harness): Harness {
  if (agents.some((a) => a.harness === brain)) return brain;
  return agents[0]?.harness ?? "claude-code";
}
