import { afterEach, describe, expect, it, vi } from "vitest";
import { renderMarkdown } from "./markdown";
import styles from "./styles.css?raw";

describe("renderMarkdown", () => {
  it("renders common markdown into elements", () => {
    const el = renderMarkdown("# Title\n\nSome **bold** and `code`.\n\n- one\n- two\n\n```sh\nls -la\n```\n\n[docs](https://example.com/x)");
    expect(el.querySelector("h1")?.textContent).toBe("Title");
    expect(el.querySelector("strong")?.textContent).toBe("bold");
    expect(el.querySelector("p code")?.textContent).toBe("code");
    expect([...el.querySelectorAll("li")].map((li) => li.textContent)).toEqual(["one", "two"]);
    expect(el.querySelector("pre code")?.textContent).toBe("ls -la\n");
    const a = el.querySelector("a")!;
    expect(a.getAttribute("href")).toBe("https://example.com/x");
  });

  it("strips scripts, event handlers and javascript links", () => {
    const el = renderMarkdown('hi <script>alert(1)</script> <img src=x onerror="alert(1)"> [x](javascript:alert(1)) <a href="https://ok" onclick="alert(1)">ok</a>');
    expect(el.querySelector("script")).toBeNull();
    expect(el.querySelector("img")).toBeNull();
    expect(el.innerHTML).not.toContain("onerror");
    expect(el.innerHTML).not.toContain("onclick");
    expect([...el.querySelectorAll("a")].every((a) => !(a.getAttribute("href") ?? "").startsWith("javascript:"))).toBe(true);
    expect(el.textContent).toContain("hi");
  });

  it("drops remote media and form widgets", () => {
    const el = renderMarkdown('<video poster="https://evil/p"></video><audio autoplay src="https://evil/a.mp3"></audio><picture><source srcset="https://evil/s"></picture><select><option>x</option></select><textarea>t</textarea><span srcset="https://evil/s" poster="https://evil/p">ok</span>');
    expect(el.querySelector("video, audio, source, picture, track, select, textarea")).toBeNull();
    expect(el.innerHTML).not.toContain("evil");
  });

  it("keeps plain text plain, with line breaks", () => {
    const el = renderMarkdown("line one\nline two");
    expect(el.textContent).toContain("line one");
    expect(el.querySelector("br")).not.toBeNull();
  });

  it("colours a code block whose fence names its language", () => {
    const el = renderMarkdown("```sql\nSELECT name FROM users WHERE id = 'x';\n```");
    const code = el.querySelector("pre code")!;
    expect(code.querySelector(".hljs-keyword")?.textContent).toBe("SELECT");
    expect(code.querySelector(".hljs-string")?.textContent).toBe("'x'");
    expect(code.textContent).toBe("SELECT name FROM users WHERE id = 'x';\n");
  });

  it("knows a language by its usual short names", () => {
    for (const lang of ["sh", "ts", "py", "rs", "yml"]) {
      const el = renderMarkdown("```" + lang + "\nx\n```");
      expect(el.querySelector("pre code")?.classList.contains("hljs"), lang).toBe(true);
    }
  });

  it("leaves a code block with no language plain", () => {
    const el = renderMarkdown("```\nSELECT 1;\n```");
    expect(el.querySelector("pre code span")).toBeNull();
    expect(el.querySelector("pre code")?.textContent).toBe("SELECT 1;\n");
  });

  it("leaves a code block in a language it does not know plain and escaped", () => {
    const el = renderMarkdown("```nosuchlang\n<b>x</b>\n```");
    expect(el.querySelector("pre code span")).toBeNull();
    expect(el.querySelector("pre code b")).toBeNull();
    expect(el.querySelector("pre code")?.textContent).toBe("<b>x</b>\n");
  });

  it("keeps markup in a coloured code block as text", () => {
    const el = renderMarkdown('```html\n<script>alert(1)</script><img src=x onerror="alert(1)">\n```');
    expect(el.querySelector("script")).toBeNull();
    expect(el.querySelector("img")).toBeNull();
    expect(el.querySelector("pre code")?.textContent).toBe('<script>alert(1)</script><img src=x onerror="alert(1)">\n');
    expect(el.querySelector("pre code .hljs-tag")).not.toBeNull();
  });

  // A code block that scrolls sideways is a WebKit trap: when its scrollbar
  // appears after the first layout (the history's own scrollbar narrowing it,
  // say) the block grows but the line below it does not move, and the block
  // paints over that line. Long lines wrap instead, as a terminal's do.
  it("wraps a code block's long lines rather than scrolling them sideways", () => {
    const rule = styles.split("\n").find((line) => line.startsWith(":is(.turn__text, .chat__turn) .md pre {")) ?? "";
    expect(rule).toContain("white-space: pre-wrap;");
    expect(rule).toContain("overflow-wrap: anywhere;");
    expect(rule).not.toContain("overflow-x");
  });
});

describe("copying a code block", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  function withClipboard(): ReturnType<typeof vi.fn> {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    return writeText;
  }

  it("puts a Copy button on every code block, and none on inline code", () => {
    const el = renderMarkdown("some `inline` code\n\n```sh\nls\n```\n\n```\npwd\n```");
    const buttons = el.querySelectorAll("pre > button.md__copy");
    expect(buttons).toHaveLength(2);
    expect(el.querySelectorAll("button")).toHaveLength(2);
    for (const b of buttons) {
      expect(b.getAttribute("type")).toBe("button");
      expect(b.getAttribute("aria-label")).toBe("Copy code");
      expect(b.textContent).toBe("Copy");
    }
  });

  it("copies the block's text, not the button's label, and says so for a moment", async () => {
    vi.useFakeTimers();
    const writeText = withClipboard();
    const el = renderMarkdown("```sql\nSELECT name FROM users WHERE id = 'x';\n```");
    const button = el.querySelector<HTMLButtonElement>("pre > button.md__copy")!;
    button.click();
    expect(writeText).toHaveBeenCalledWith("SELECT name FROM users WHERE id = 'x';\n");
    await Promise.resolve();
    expect(button.textContent).toBe("Copied");
    vi.advanceTimersByTime(1500);
    expect(button.textContent).toBe("Copy");
  });

  it("leaves the block's own text untouched by the button", () => {
    const el = renderMarkdown("```\nx\n```");
    expect(el.querySelector("pre code")?.textContent).toBe("x\n");
  });
});
