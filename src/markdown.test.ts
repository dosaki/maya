import { describe, expect, it } from "vitest";
import { renderMarkdown } from "./markdown";

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
});
