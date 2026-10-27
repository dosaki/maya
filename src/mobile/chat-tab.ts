import type { NetworkStatus } from "./network";

/** Chat shows only while the phone is joined to a main that lets it mirror; otherwise the tab hides. */
export function chatTabShown(s: NetworkStatus): boolean {
  return s.role === "assistant" && s.assistant.mirror === "allowed";
}
