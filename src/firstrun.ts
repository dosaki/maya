import { invoke } from "@tauri-apps/api/core";
import { HARNESS_ICON, harnessLabel } from "./harness";
import { CLAUDE_AGENT, type AgentInfo } from "./newsession";
import type { Harness } from "./types";

export interface FirstRunModel {
  /** The installed agents; null while they are being listed. */
  agents: AgentInfo[] | null;
  chosen: Harness;
  saving: boolean;
  error: string | null;
}

export interface FirstRunHandlers {
  onChoose(agent: Harness): void;
  onContinue(): void;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  n.className = className;
  if (text !== undefined) n.textContent = text;
  return n;
}

/** True when no agent has been chosen yet: a fresh install, or the first start after 0.10. */
export function needsFirstRun(config: { agent?: Harness | null }): boolean {
  return config.agent == null;
}

/** The first-start question. It has no close button: Continue is the only way out. */
export function renderFirstRun(m: FirstRunModel, h: FirstRunHandlers): HTMLElement {
  const root = el("div", "modal");
  root.append(el("div", "modal__backdrop"));
  const panel = el("section", "modal__panel modal__panel--compact firstrun");
  panel.setAttribute("role", "dialog");
  panel.setAttribute("aria-modal", "true");
  const head = el("header", "modal__head");
  const titles = el("div", "modal__titles");
  titles.append(el("h2", "modal__title", "Which agent should power Maya?"));
  head.append(titles);
  panel.append(head);
  const body = el("div", "modal__setup");
  if (m.agents === null) {
    body.append(el("p", "", "Looking for agents on this machine…"));
  } else if (m.agents.length === 0) {
    body.append(el("p", "", "None of the agents Maya can use is installed: Claude Code, Codex, Antigravity or Grok Build. Install one, then pick it in Settings."));
  } else {
    body.append(el("p", "", "It interprets your voice commands, picks folders for 'Let Maya choose', and is the default for new and resumed sessions and reviews. You can change it in Settings."));
    const list = el("div", "resume__list");
    for (const a of m.agents) {
      const row = el("label", "firstrun__row");
      const radio = document.createElement("input");
      radio.type = "radio";
      radio.name = "agent";
      radio.value = a.harness;
      radio.checked = a.harness === m.chosen;
      radio.addEventListener("click", () => h.onChoose(a.harness));
      row.append(radio);
      const icon = HARNESS_ICON[a.harness];
      if (icon) {
        const img = document.createElement("img");
        img.src = icon;
        img.alt = "";
        img.width = 20;
        img.height = 20;
        row.append(img);
      }
      row.append(el("span", "", harnessLabel(a.harness)));
      list.append(row);
    }
    body.append(list);
  }
  if (m.agents !== null) {
    const go = el("button", "card__btn card__btn--primary", m.agents.length === 0 ? "Continue with Claude Code" : "Continue");
    go.type = "button";
    go.dataset.action = "continue";
    go.disabled = m.saving;
    go.addEventListener("click", () => h.onContinue());
    body.append(go);
  }
  panel.append(body);
  if (m.error) panel.append(el("div", "modal__status modal__status--error", m.error));
  root.append(panel);
  return root;
}

interface ConfigLike {
  agent?: Harness | null;
  agentModel?: string;
  [key: string]: unknown;
}

/** On a start with no agent chosen, asks which one, saves it, and closes. */
export async function maybeShowFirstRun(): Promise<void> {
  let config: ConfigLike;
  try {
    config = await invoke<ConfigLike>("get_config");
  } catch {
    return;
  }
  if (!needsFirstRun(config)) return;
  const host = document.getElementById("modal-host");
  if (!host) return;
  const model: FirstRunModel = { agents: null, chosen: "claude-code", saving: false, error: null };
  await new Promise<void>((resolve) => {
    const paint = () => host.replaceChildren(renderFirstRun(model, handlers));
    const handlers: FirstRunHandlers = {
      onChoose: (agent) => {
        model.chosen = agent;
      },
      onContinue: () => {
        void (async () => {
          model.saving = true;
          model.error = null;
          paint();
          try {
            const fresh = await invoke<ConfigLike>("get_config");
            const agentModel = model.chosen === "claude-code" ? (fresh.agentModel ?? "") : "";
            await invoke("set_config", { config: { ...fresh, agent: model.chosen, agentModel } });
            host.replaceChildren();
            resolve();
          } catch (e) {
            model.saving = false;
            model.error = String(e);
            paint();
          }
        })();
      },
    };
    paint();
    void invoke<{ agents: AgentInfo[] }>("list_agents", { machine: "" })
      .then((r) => {
        model.agents = r.agents;
        model.chosen = r.agents[0]?.harness ?? "claude-code";
      })
      .catch(() => {
        model.agents = [CLAUDE_AGENT];
        model.chosen = "claude-code";
      })
      .then(paint);
  });
}
