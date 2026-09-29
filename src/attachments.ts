/** A file the next message will point the session at. */
export interface Attachment {
  name: string;
  /** Absolute path the session can read. */
  path: string;
}

/** A pasted file with the name it will be saved under. */
export interface PastedFile {
  name: string;
  file: File;
}

export function makeAttachments() {
  let items: Attachment[] = [];
  return {
    list: () => items.slice(),
    add: (a: Attachment) => {
      if (!items.some((x) => x.path === a.path)) items = [...items, a];
    },
    remove: (path: string) => {
      items = items.filter((x) => x.path !== path);
    },
    clear: () => {
      items = [];
    },
  };
}

/** The message text plus one "Attached file:" line per attachment. */
export function composeMessage(text: string, attachments: Attachment[]): string {
  const body = text.trim();
  const lines = attachments.map((a) => `Attached file: ${a.path}`).join("\n");
  if (!lines) return body;
  return body ? `${body}\n\n${lines}` : lines;
}

export function renderChips(attachments: Attachment[], onRemove: (path: string) => void): HTMLElement {
  const root = document.createElement("div");
  root.className = "modal__chips";
  for (const a of attachments) {
    const chip = document.createElement("span");
    chip.className = "chip";
    chip.title = a.path;
    const name = document.createElement("span");
    name.className = "chip__name";
    name.textContent = a.name;
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "chip__remove";
    remove.textContent = "×";
    remove.setAttribute("aria-label", `Remove ${a.name}`);
    remove.addEventListener("click", () => onRemove(a.path));
    chip.append(name, remove);
    root.append(chip);
  }
  return root;
}

const EXT: Record<string, string> = { "image/png": "png", "image/jpeg": "jpg", "image/gif": "gif", "image/webp": "webp" };

/**
 * The files among pasted clipboard items. A pasted image usually arrives as
 * an unnamed "image.png"; it gets a timestamped name so saves do not collide.
 */
export function pastedFiles(items: ArrayLike<{ kind: string; type: string; getAsFile(): File | null }>, nowMs: number): PastedFile[] {
  const out: PastedFile[] = [];
  for (const item of Array.from(items)) {
    if (item.kind !== "file") continue;
    const file = item.getAsFile();
    if (!file) continue;
    const generic = !file.name || file.name === "image.png" || file.name.startsWith("image.");
    const name = generic && file.type.startsWith("image/") ? `pasted-${nowMs}.${EXT[file.type] ?? "png"}` : file.name || `pasted-${nowMs}`;
    out.push({ name, file });
  }
  return out;
}
