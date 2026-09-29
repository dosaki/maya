import DOMPurify from "dompurify";
import { marked } from "marked";

marked.setOptions({ gfm: true, breaks: true });

/**
 * Markdown to a sanitised element. Scripts, event handlers, images and
 * javascript: links are dropped; what remains is inert text and links.
 */
export function renderMarkdown(text: string): HTMLElement {
  const root = document.createElement("div");
  root.className = "md";
  const html = marked.parse(text, { async: false }) as string;
  root.innerHTML = DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true },
    FORBID_TAGS: ["img", "svg", "style", "form", "input", "button", "iframe", "object", "embed"],
    FORBID_ATTR: ["style"],
  });
  for (const a of root.querySelectorAll("a")) {
    a.setAttribute("rel", "noopener noreferrer");
    const href = a.getAttribute("href") ?? "";
    if (!/^https?:\/\//i.test(href)) a.removeAttribute("href");
  }
  return root;
}
