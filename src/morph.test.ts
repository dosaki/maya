import { describe, expect, it } from "vitest";
import { keepEqualChildren, morph } from "./morph";

const html = (s: string): HTMLElement => {
  const t = document.createElement("template");
  t.innerHTML = s.trim();
  return t.content.firstElementChild as HTMLElement;
};

describe("morph", () => {
  it("brings attributes and text in line, keeping the old nodes", () => {
    const old = html(`<div class="a" title="t"><span>one</span> tail</div>`);
    const span = old.firstElementChild;
    morph(old, html(`<div class="b" data-x="1"><span>two</span> end</div>`));
    expect(old.outerHTML).toBe(`<div class="b" data-x="1"><span>two</span> end</div>`);
    expect(old.firstElementChild).toBe(span);
  });

  it("swaps a child whose tag changed", () => {
    const old = html(`<div><span>x</span></div>`);
    morph(old, html(`<div><button>x</button></div>`));
    expect(old.innerHTML).toBe("<button>x</button>");
  });

  it("matches keyed children by key, so a reorder moves nodes rather than rewriting them", () => {
    const old = html(`<ul><li data-session-id="a">A</li><li data-session-id="b">B</li><li data-session-id="c">C</li></ul>`);
    const [a, , c] = [...old.children];
    morph(old, html(`<ul><li data-session-id="c">C</li><li data-session-id="d">D</li><li data-session-id="a">A2</li></ul>`));
    expect(old.innerHTML).toBe(`<li data-session-id="c">C</li><li data-session-id="d">D</li><li data-session-id="a">A2</li>`);
    expect(old.children[0]).toBe(c);
    expect(old.children[2]).toBe(a);
  });

  it("never pairs a keyed node with an unkeyed one", () => {
    const old = html(`<div><p>empty</p></div>`);
    const p = old.firstElementChild;
    morph(old, html(`<div><p data-session-id="a">A</p></div>`));
    expect(old.firstElementChild).not.toBe(p);
    expect(old.innerHTML).toBe(`<p data-session-id="a">A</p>`);
  });
});

describe("keepEqualChildren", () => {
  it("keeps each child equal to its fresh counterpart and takes the fresh node for the rest", () => {
    const old = html(`<div><p>1</p><p>2</p><p>3</p></div>`);
    const [one, two] = [...old.children];
    const fresh = html(`<div><p>1</p><p>2</p><p>3!</p><p>4</p></div>`);
    const [, , three, four] = [...fresh.children];
    keepEqualChildren(old, fresh);
    expect(old.innerHTML).toBe(`<p>1</p><p>2</p><p>3!</p><p>4</p>`);
    expect(old.children[0]).toBe(one);
    expect(old.children[1]).toBe(two);
    expect(old.children[2]).toBe(three);
    expect(old.children[3]).toBe(four);
  });

  it("patches inside the children `within` picks rather than swapping them", () => {
    const old = html(`<div><ol class="log"><li>1</li></ol><p>bar</p></div>`);
    const [log, bar] = [...old.children];
    const one = log.firstElementChild;
    keepEqualChildren(old, html(`<div><ol class="log"><li>1</li><li>2</li></ol><p>bar!</p></div>`), (el) => el.classList.contains("log"));
    expect(old.innerHTML).toBe(`<ol class="log"><li>1</li><li>2</li></ol><p>bar!</p>`);
    expect(old.children[0]).toBe(log);
    expect(log.firstElementChild).toBe(one);
    expect(old.children[1]).not.toBe(bar);
  });

  it("drops children the fresh render no longer has", () => {
    const old = html(`<div><p>1</p><p>2</p></div>`);
    keepEqualChildren(old, html(`<div><p>1</p></div>`));
    expect(old.innerHTML).toBe(`<p>1</p>`);
  });
});
