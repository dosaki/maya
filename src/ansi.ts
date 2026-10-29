/** The sixteen standard colours: 0-7 normal, 8-15 bright. */
const SIXTEEN = [
  "#000000", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5",
  "#666666", "#f14c4c", "#23d18b", "#f5f543", "#3b8eea", "#d670d6", "#29b8db", "#ffffff",
];

const rgb = (r: number, g: number, b: number) => `rgb(${r}, ${g}, ${b})`;
const clamp = (n: number) => Math.max(0, Math.min(255, n));

/** A 256-colour index as a CSS colour, or null when out of range. */
function palette(n: number): string | null {
  if (!Number.isInteger(n) || n < 0 || n > 255) return null;
  if (n < 16) return SIXTEEN[n];
  if (n < 232) {
    const i = n - 16;
    const level = (v: number) => (v === 0 ? 0 : 55 + v * 40);
    return rgb(level(Math.floor(i / 36)), level(Math.floor(i / 6) % 6), level(i % 6));
  }
  const g = 8 + (n - 232) * 10;
  return rgb(g, g, g);
}

interface Style {
  bold: boolean;
  dim: boolean;
  fg: string | null;
  bg: string | null;
}

const PLAIN = (): Style => ({ bold: false, dim: false, fg: null, bg: null });

/** Applies one SGR parameter list to the style. */
function applySgr(style: Style, params: string): void {
  const p = params === "" ? [0] : params.split(";").map((s) => (s === "" ? 0 : Number(s)));
  for (let i = 0; i < p.length; i++) {
    const n = p[i];
    if (n === 0) Object.assign(style, PLAIN());
    else if (n === 1) style.bold = true;
    else if (n === 2) style.dim = true;
    else if (n === 22) {
      style.bold = false;
      style.dim = false;
    } else if (n >= 30 && n <= 37) style.fg = SIXTEEN[n - 30];
    else if (n >= 90 && n <= 97) style.fg = SIXTEEN[n - 90 + 8];
    else if (n === 39) style.fg = null;
    else if (n >= 40 && n <= 47) style.bg = SIXTEEN[n - 40];
    else if (n >= 100 && n <= 107) style.bg = SIXTEEN[n - 100 + 8];
    else if (n === 49) style.bg = null;
    else if (n === 38 || n === 48) {
      let colour: string | null = null;
      if (p[i + 1] === 5) {
        colour = palette(p[i + 2]);
        i += 2;
      } else if (p[i + 1] === 2) {
        const [r, g, b] = [p[i + 2], p[i + 3], p[i + 4]];
        if ([r, g, b].every((v) => Number.isFinite(v))) colour = rgb(clamp(r), clamp(g), clamp(b));
        i += 4;
      }
      if (n === 38) style.fg = colour;
      else style.bg = colour;
    }
  }
}

type Piece = { text: string } | { sgr: string };

/** Splits the text into plain runs and SGR parameter lists; every other escape sequence is dropped. */
function tokens(text: string): Piece[] {
  const out: Piece[] = [];
  let run = "";
  const flush = () => {
    if (run) out.push({ text: run });
    run = "";
  };
  let i = 0;
  while (i < text.length) {
    const c = text[i];
    if (c !== "\u001b") {
      run += c;
      i++;
      continue;
    }
    const next = text[i + 1];
    if (next === undefined) break; // a lone trailing ESC
    if (next === "[") {
      let j = i + 2;
      while (j < text.length && !(text.charCodeAt(j) >= 0x40 && text.charCodeAt(j) <= 0x7e)) j++;
      if (j >= text.length) break; // unterminated: drop the rest
      if (text[j] === "m") {
        flush();
        out.push({ sgr: text.slice(i + 2, j) });
      }
      i = j + 1;
    } else if (next === "]") {
      let j = i + 2;
      while (j < text.length && text[j] !== "\u0007" && !(text[j] === "\u001b" && text[j + 1] === "\\")) j++;
      i = j >= text.length ? j : text[j] === "\u0007" ? j + 1 : j + 2;
    } else {
      i += 2;
    }
  }
  flush();
  return out;
}

/** Text with SGR colour rendered as <span> runs (classes `ansi-b`, `ansi-d`, inline `color` and `background-color` for 16/256/truecolor), every other escape sequence removed (CSI, OSC up to BEL or ST, lone ESC). Never inserts markup from the text itself. */
export function renderAnsi(text: string): DocumentFragment {
  const frag = document.createDocumentFragment();
  const style = PLAIN();
  for (const piece of tokens(text)) {
    if ("sgr" in piece) {
      applySgr(style, piece.sgr);
      continue;
    }
    if (!style.bold && !style.dim && !style.fg && !style.bg) {
      frag.append(document.createTextNode(piece.text));
      continue;
    }
    const span = document.createElement("span");
    if (style.bold) span.classList.add("ansi-b");
    if (style.dim) span.classList.add("ansi-d");
    if (style.fg) span.style.color = style.fg;
    if (style.bg) span.style.backgroundColor = style.bg;
    span.append(document.createTextNode(piece.text));
    frag.append(span);
  }
  return frag;
}

/** The same text with every escape sequence removed. */
export function stripAnsi(text: string): string {
  return tokens(text).map((p) => ("text" in p ? p.text : "")).join("");
}
