// What's new after an update: the release notes since the version this
// device last showed, with Maya presenting them. Shared with the phone, so
// nothing here may import desktop-only code; `styles.css` gives it the
// computer's look and `mobile/mobile.css` the phone's.

import art from "./assets/maya-full-body.png";
import { RELEASE_NOTES, renderReleaseNotes, type ReleaseNotes } from "./release-notes";

/** What the backend's `whats_new` answers: the notes after `from` up to `running`; with no `from`, only `running`'s. */
export interface WhatsNewOffer {
  from: string | null;
  running: string;
}

function parse(v: string): [number, number, number] | null {
  const m = /^(\d+)\.(\d+)\.(\d+)$/.exec(v.trim());
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

/** Below, at or above zero as `a` is older than, equal to or newer than `b`, by number; null when either is not a version. */
export function compareVersions(a: string, b: string): number | null {
  const x = parse(a);
  const y = parse(b);
  if (!x || !y) return null;
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] - y[i];
  return 0;
}

/** The sections `offer` covers, in the file's order (newest first). */
export function notesSince(offer: WhatsNewOffer, notes: ReleaseNotes[] = RELEASE_NOTES): ReleaseNotes[] {
  return notes.filter(({ version }) => {
    const toRunning = compareVersions(version, offer.running);
    if (toRunning === null) return false;
    if (offer.from === null) return toRunning === 0;
    const toFrom = compareVersions(version, offer.from);
    return toFrom !== null && toFrom > 0 && toRunning <= 0;
  });
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/** The window: `onClose` runs on Got it, the × and a click on the backdrop. Escape is the caller's. */
export function renderWhatsNew(offer: WhatsNewOffer, notes: ReleaseNotes[], onClose: () => void): HTMLElement {
  const root = el("div", "modal whatsnew");
  root.setAttribute("role", "dialog");
  root.setAttribute("aria-modal", "true");
  root.setAttribute("aria-labelledby", "whatsnew-title");
  const backdrop = el("div", "modal__backdrop");
  backdrop.addEventListener("click", onClose);
  const panel = el("div", "modal__panel whatsnew__panel");

  const img = el("img", "whatsnew__art");
  img.src = art;
  img.alt = "";

  const close = el("button", "whatsnew__close", "×");
  close.type = "button";
  close.setAttribute("aria-label", "Close");
  close.addEventListener("click", onClose);

  const head = el("div", "whatsnew__head");
  if (offer.from !== null) head.append(el("div", "whatsnew__kicker", `Updated from ${offer.from}`));
  const title = el("h2", "whatsnew__title", `What's new in Maya ${offer.running}`);
  title.id = "whatsnew-title";
  head.append(title);
  if (offer.from !== null) {
    const n = notes.length;
    head.append(el("p", "whatsnew__count", `${n} ${n === 1 ? "version" : "versions"} since you last opened Maya.`));
  }

  const body = el("div", "whatsnew__notes");
  body.append(renderReleaseNotes(notes));
  for (const h of body.querySelectorAll<HTMLElement>(".release-notes__version")) {
    if (h.textContent === `Maya ${offer.running}`) h.append(" ", el("span", "whatsnew__chip", "This version"));
  }

  const foot = el("div", "whatsnew__foot");
  const ok = el("button", "whatsnew__ok", "Got it");
  ok.type = "button";
  ok.addEventListener("click", onClose);
  foot.append(el("span", "whatsnew__where", "All release notes are in Settings › Release Notes."), ok);

  panel.append(img, close, head, body, foot);
  root.append(backdrop, panel);
  return root;
}

/** The open window's close, if one is open. */
let closeOpen: ((byBack?: boolean) => void) | null = null;

/**
 * Asks the backend whether to show the window and shows it in `host`.
 * Nothing shows when the answer is null, or when `host` already holds
 * something (a card opened at start): it waits for the next start, unseen.
 * Closing it in any way empties `host` and tells the backend it was seen.
 * With `history` (the phone), it holds one entry while open, so Back
 * closes it instead of leaving the app.
 */
export async function showWhatsNew(invoke: (cmd: string) => Promise<unknown>, host: HTMLElement | null, notes: ReleaseNotes[] = RELEASE_NOTES, history?: History): Promise<void> {
  if (!host) return;
  const offer = (await invoke("whats_new")) as WhatsNewOffer | null;
  if (!offer || host.childElementCount > 0 || closeOpen) return;
  const seen = () => void invoke("whats_new_seen").catch((e) => console.warn("could not save that What's new was seen:", e));
  const shown = notesSince(offer, notes);
  if (shown.length === 0) {
    seen();
    return;
  }
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") close();
  };
  const onBack = () => close(true);
  const close = (byBack = false) => {
    if (closeOpen !== close) return;
    closeOpen = null;
    document.removeEventListener("keydown", onKey);
    if (history) {
      window.removeEventListener("popstate", onBack);
      // Closed some other way: its entry goes, so Back is never needed twice.
      if (!byBack && (history.state as { whatsNew?: unknown } | null)?.whatsNew) history.back();
    }
    // A card that took its place keeps the host, and the window was not
    // read: it shows again at the next start.
    if (!root.isConnected) return;
    host.replaceChildren();
    seen();
  };
  const root = renderWhatsNew(offer, shown, () => close());
  closeOpen = close;
  host.replaceChildren(root);
  document.addEventListener("keydown", onKey);
  if (history) {
    history.pushState({ whatsNew: true }, "");
    window.addEventListener("popstate", onBack);
  }
}

/** Closes the window, if open, as Got it does: a tab tap on the phone. */
export function closeWhatsNew(): void {
  closeOpen?.();
}
