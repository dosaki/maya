// Whether this Maya runs sessions of its own, and what the Machine pickers
// list. The desktop does; the phone does not, so its pickers list the
// connected assistants alone and a dialog with none says to pair one.

import { thisComputer } from "./platform";

export interface MachineChoice {
  name: string;
  value: string;
}

/** One row of `list_machines`. */
export interface MachineInfo {
  name: string;
  hostname: string;
  platform: string;
  connected: boolean;
}

let local = true;

/** Off on the phone, which has no sessions of its own. */
export function setLocalMachine(on: boolean): void {
  local = on;
}

export function hasLocalMachine(): boolean {
  return local;
}

/** "This Mac" first when this machine runs sessions, then each connected assistant. */
export function machineChoices(machines: MachineInfo[]): MachineChoice[] {
  const remote = machines.filter((m) => m.connected).map((m) => ({ name: m.name, value: m.name }));
  return local ? [{ name: thisComputer(), value: "" }, ...remote] : remote;
}

/** What a dialog shows before `list_machines` answers. */
export function initialChoices(): MachineChoice[] {
  return local ? [{ name: thisComputer(), value: "" }] : [];
}
