import styles from "./styles.css?raw";
import { beforeEach, describe, expect, it } from "vitest";
import { PREVIEW_LINES, renderToolTurn, resetToolUi, rowText, toolHead } from "./toolturn";
import type { Turn } from "./types";

const lines = (n: number) => Array.from({ length: n }, (_, i) => `line ${i + 1}`).join("\n");
const bash: Turn = { kind: "tool", text: "Bash(cargo test)", output: lines(10), outputLines: 10 };
const edit: Turn = {
  kind: "tool",
  text: "Edit(/x/src/a.rs)",
  diff: [
    { op: "del", line: 3, text: "fn mian() {" },
    { op: "add", line: 3, text: "fn main() {" },
    { op: "same", line: 4, text: "    run();" },
    { op: "add", line: 5, text: "    done();" },
  ],
  added: 2,
  removed: 1,
};

beforeEach(resetToolUi);

describe("tool turns", () => {
  it("reads the call's name and argument, in this Maya's form and an older one's", () => {
    expect(toolHead("Bash(cargo test)")).toEqual({ name: "Bash", arg: "cargo test" });
    expect(toolHead("Bash: cargo test")).toEqual({ name: "Bash", arg: "cargo test" });
    expect(toolHead("mcp__pr-watch__wait_for_pr(37)")).toEqual({ name: "mcp__pr-watch__wait_for_pr", arg: "37" });
    expect(toolHead("ListAgents")).toEqual({ name: "ListAgents", arg: "" });
  });

  it("shows the first 4 lines under ⎿ and expands the rest in place", () => {
    const t = renderToolTurn(bash);
    expect(t.querySelector(".turn__mark")?.textContent).toBe("⏺");
    expect(t.querySelector(".turn__text")?.textContent).toBe("Bash(cargo test)");
    expect(t.querySelector(".turn__elbow")?.textContent).toBe("⎿");
    expect(t.querySelector(".turn__out")?.textContent).toBe(lines(PREVIEW_LINES));
    const more = t.querySelector<HTMLButtonElement>(".turn__more")!;
    expect(more.textContent).toBe("… +6 lines");
    more.click();
    expect(t.querySelector(".turn__out")?.textContent).toBe(lines(10));
    expect(t.querySelector(".turn__more")).toBeNull();
  });

  it("clamps the collapsed preview's height, and not the expanded result's", () => {
    const t = renderToolTurn(bash);
    expect(t.querySelector(".turn__out")?.classList.contains("turn__out--preview")).toBe(true);
    t.querySelector<HTMLButtonElement>(".turn__more")!.click();
    expect(t.querySelector(".turn__out")?.classList.contains("turn__out--preview")).toBe(false);
    // Shown whole: nothing to expand into, so nothing is clamped away.
    const short = renderToolTurn({ kind: "tool", text: "Bash(ls)", output: "a.rs", outputLines: 1 });
    expect(short.querySelector(".turn__out")?.classList.contains("turn__out--preview")).toBe(false);
    expect(styles).toContain(".turn__out--preview { max-height: calc(4 * 1.45em); overflow: hidden; }");
  });

  it("counts lines the cap left out, and keeps an expanded result open across repaints", () => {
    const capped: Turn = { ...bash, output: lines(40), outputLines: 250 };
    const t = renderToolTurn(capped);
    expect(t.querySelector(".turn__more")?.textContent).toBe("… +246 lines");
    t.querySelector<HTMLButtonElement>(".turn__more")!.click();
    expect(t.querySelector(".turn__left")?.textContent).toBe("… +210 lines");
    const again = renderToolTurn({ ...capped });
    expect(again.querySelector(".turn__out")?.textContent).toBe(lines(40));
    expect(again.querySelector(".turn__more")).toBeNull();
    resetToolUi();
    expect(renderToolTurn({ ...capped }).querySelector(".turn__more")).not.toBeNull();
  });

  it("shows a short result whole, and no result without output", () => {
    const short = renderToolTurn({ kind: "tool", text: "Bash(ls)", output: "a.rs\nb.rs", outputLines: 2 });
    expect(short.querySelector(".turn__out")?.textContent).toBe("a.rs\nb.rs");
    expect(short.querySelector(".turn__more")).toBeNull();
    expect(renderToolTurn({ kind: "tool", text: "ListAgents" }).querySelector(".turn__result")).toBeNull();
  });

  it("draws an edit as Update(path) with its counts and coloured, numbered lines", () => {
    const t = renderToolTurn(edit);
    expect(t.querySelector(".turn__text")?.textContent).toBe("Update(/x/src/a.rs)");
    expect(t.querySelector(".turn__summary-text")?.textContent).toBe("Updated /x/src/a.rs (+2 −1)");
    const rows = [...t.querySelectorAll(".diff__line")].map((r) => [r.className, r.querySelector(".diff__no")?.textContent, r.querySelector(".diff__sign")?.textContent, r.querySelector(".diff__text")?.textContent]);
    expect(rows).toEqual([
      ["diff__line diff__line--del", "3", "-", "fn mian() {"],
      ["diff__line diff__line--add", "3", "+", "fn main() {"],
      ["diff__line diff__line--same", "4", " ", "    run();"],
      ["diff__line diff__line--add", "5", "+", "    done();"],
    ]);
    expect(styles).toContain(".diff__line--add {");
    expect(styles).toContain(".diff__line--del {");
  });

  it("leaves a diff line's number blank when unknown, and says how many lines a cap left out", () => {
    const t = renderToolTurn({ kind: "tool", text: "Write(/x/big.txt)", diff: [{ op: "add", text: "first" }], added: 300, removed: 0 });
    expect(t.querySelector(".diff__no")?.textContent).toBe("");
    expect(t.querySelector(".turn__diff .turn__left")?.textContent).toBe("… +299 lines");
  });

  it("marks a failed call red in both looks", () => {
    const t = renderToolTurn({ kind: "tool", text: "Bash(ls /nope)", output: "ls: /nope: No such file or directory", outputLines: 1, isError: true });
    expect(t.classList.contains("turn--failed")).toBe(true);
    expect(t.querySelector(".turn__row")?.textContent).toBe("▸ Bash · ls /nope ✕");
    expect(styles).toContain(".turn--failed .turn__out { color: #dc2626; }");
    expect(styles).toContain(".turn__fail { color: #dc2626; }");
  });

  it("gives the chat look a compact row that opens the output or diff", () => {
    expect(rowText(bash)).toBe("▸ Bash · cargo test");
    expect(rowText(edit)).toBe("▸ Update · /x/src/a.rs (+2 −1)");
    expect(rowText({ kind: "tool", text: "ListAgents" })).toBe("▸ ListAgents");
    const t = renderToolTurn(bash);
    const row = t.querySelector<HTMLButtonElement>(".turn__row")!;
    expect(row.textContent).toBe("▸ Bash · cargo test");
    expect(t.classList.contains("turn--open")).toBe(false);
    row.click();
    expect(t.classList.contains("turn--open")).toBe(true);
    expect(row.getAttribute("aria-expanded")).toBe("true");
    expect(renderToolTurn({ ...bash }).classList.contains("turn--open")).toBe(true);
    row.click();
    expect(renderToolTurn({ ...bash }).classList.contains("turn--open")).toBe(false);
    expect(renderToolTurn({ kind: "tool", text: "ListAgents" }).querySelector<HTMLButtonElement>(".turn__row")!.disabled).toBe(true);
    expect(styles).toContain(".turn--open > .turn__result { display: block; }");
    expect(styles).toContain(".modal--terminal .turn__row { display: none; }");
  });

  it("renders output and diff text as text, never markup", () => {
    const evil = '<img src=x onerror="alert(1)"><script>alert(2)</script>';
    const t = renderToolTurn({ kind: "tool", text: `Bash(${evil})`, output: evil, outputLines: 1 });
    expect(t.querySelector("img, script")).toBeNull();
    expect(t.querySelector(".turn__out")?.textContent).toBe(evil);
    expect(t.querySelector(".turn__text")?.textContent).toBe(`Bash(${evil})`);
    const d = renderToolTurn({ kind: "tool", text: "Edit(/x/a.html)", diff: [{ op: "add", line: 1, text: evil }], added: 1, removed: 0 });
    expect(d.querySelector("img, script")).toBeNull();
    expect(d.querySelector(".diff__text")?.textContent).toBe(evil);
  });
});
