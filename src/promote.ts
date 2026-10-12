// The confirmation shown before a main hands its role to an assistant:
// what will happen, who cannot follow, and who is offline.

import type { AssistantStatus } from "./settings";

export function promoteLines(assistants: AssistantStatus[], id: string, thisDevice: string): string[] {
  const g = assistants.find((a) => a.id === id);
  if (!g) return [];
  const others = assistants.filter((a) => a.id !== id);
  return [
    `Make ${g.name} the main Maya? Every connected Maya will assist ${g.name}, and so will ${thisDevice}.`,
    ...others.filter((a) => a.connected && !a.canFollow).map((a) => `${a.name} cannot follow and stays with no main; pair it with ${g.name} afterwards.`),
    ...others.filter((a) => !a.connected).map((a) => `${a.name} is offline; when it is back, set its Host to ${g.address}. It does not need to pair again.`),
  ];
}
