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

  it("keeps plain text plain, with line breaks", () => {
    const el = renderMarkdown("line one\nline two");
    expect(el.textContent).toContain("line one");
    expect(el.querySelector("br")).not.toBeNull();
  });
});
