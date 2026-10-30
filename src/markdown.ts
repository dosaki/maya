import DOMPurify from "dompurify";
import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import css from "highlight.js/lib/languages/css";
import diff from "highlight.js/lib/languages/diff";
import dockerfile from "highlight.js/lib/languages/dockerfile";
import go from "highlight.js/lib/languages/go";
import ini from "highlight.js/lib/languages/ini";
import java from "highlight.js/lib/languages/java";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import kotlin from "highlight.js/lib/languages/kotlin";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import shell from "highlight.js/lib/languages/shell";
import sql from "highlight.js/lib/languages/sql";
import swift from "highlight.js/lib/languages/swift";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";
import { marked } from "marked";

/** The languages agents write most; each also answers to its short names (sh, ts, py, yml, html, toml through ini). */
const LANGUAGES = { bash, css, diff, dockerfile, go, ini, java, javascript, json, kotlin, python, rust, shell, sql, swift, typescript, xml, yaml };
for (const [name, language] of Object.entries(LANGUAGES)) hljs.registerLanguage(name, language);

marked.setOptions({ gfm: true, breaks: true });
marked.use({
  renderer: {
    /**
     * A fence that names a language Maya knows is coloured; any other is left
     * to marked, plain. No guessing: a guess colours logs and output oddly.
     */
    code({ text, lang }) {
      const name = (lang ?? "").trim().split(/\s+/)[0].toLowerCase();
      if (!name || !hljs.getLanguage(name)) return false;
      const html = hljs.highlight(text, { language: name, ignoreIllegals: true }).value;
      return `<pre><code class="hljs language-${name}">${html}\n</code></pre>\n`;
    },
  },
});

/**
 * Markdown to a sanitised element. Scripts, event handlers, images and
 * other remote media, form widgets and javascript: links are dropped; what remains is inert text and links.
 */
export function renderMarkdown(text: string): HTMLElement {
  const root = document.createElement("div");
  root.className = "md";
  const html = marked.parse(text, { async: false }) as string;
  root.innerHTML = DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true },
    FORBID_TAGS: ["img", "svg", "style", "form", "input", "button", "select", "textarea", "iframe", "object", "embed", "video", "audio", "source", "picture", "track"],
    FORBID_ATTR: ["style", "poster", "srcset"],
  });
  for (const a of root.querySelectorAll("a")) {
    a.setAttribute("rel", "noopener noreferrer");
    const href = a.getAttribute("href") ?? "";
    if (!/^https?:\/\//i.test(href)) a.removeAttribute("href");
  }
  // Added after sanitising, which drops every button the text might carry.
  for (const pre of root.querySelectorAll("pre")) pre.append(copyButton(pre));
  return root;
}

/** How long the button says "Copied" before it offers to copy again. */
const COPIED_MS = 1500;

/** A Copy button for a code block, in its corner: a click puts the block's text on the clipboard. */
function copyButton(pre: HTMLPreElement): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "md__copy";
  button.setAttribute("aria-label", "Copy code");
  button.textContent = "Copy";
  let restore = 0;
  button.addEventListener("click", () => {
    const text = pre.querySelector("code")?.textContent ?? "";
    void navigator.clipboard?.writeText(text).then(() => {
      button.textContent = "Copied";
      window.clearTimeout(restore);
      restore = window.setTimeout(() => { button.textContent = "Copy"; }, COPIED_MS);
    });
  });
  return button;
}
