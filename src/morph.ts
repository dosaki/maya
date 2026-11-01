// Repainting in place. A replaced node loses what the browser keeps on it:
// WebKit drops :hover until the pointer moves, and a tooltip, a text
// selection, focus and a CSS transition go with the node. So a repaint
// changes the nodes already on screen wherever the fresh render matches them.

/** A card's session id: keyed children are matched by it rather than by place. */
const keyOf = (n: Node): string | null => (n instanceof Element ? n.getAttribute("data-session-id") : null);

const sameKind = (a: Node, b: Node): boolean =>
  a.nodeType === b.nodeType && (!(a instanceof Element) || a.tagName === (b as Element).tagName);

/**
 * Makes `old` read as `fresh`, keeping every old node the fresh render has a
 * counterpart for: the same session id, or for an unkeyed node the next one
 * of the same kind in order. For trees without listeners of their own (the
 * board delegates its clicks): a kept node keeps its old listeners.
 */
export function morph(old: Element, fresh: Element): void {
  for (const { name } of [...old.attributes]) if (!fresh.hasAttribute(name)) old.removeAttribute(name);
  for (const { name, value } of fresh.attributes) if (old.getAttribute(name) !== value) old.setAttribute(name, value);

  const keyed = new Map<string, Node>();
  const unkeyed: Node[] = [];
  for (const n of old.childNodes) {
    const k = keyOf(n);
    if (k === null) unkeyed.push(n);
    else keyed.set(k, n);
  }
  let next = 0;
  const wanted = [...fresh.childNodes].map((f) => {
    const k = keyOf(f);
    const o = k === null ? unkeyed[next++] : keyed.get(k);
    if (k !== null) keyed.delete(k);
    if (!o || !sameKind(o, f)) return f;
    if (o instanceof Element) morph(o, f as Element);
    else if (o.nodeValue !== f.nodeValue) o.nodeValue = f.nodeValue;
    return o;
  });
  // Dropping the leftovers first leaves the kept nodes in place unless they really moved.
  const kept = new Set(wanted);
  for (const n of [...old.childNodes]) if (!kept.has(n)) n.remove();
  wanted.forEach((n, i) => {
    if (old.childNodes[i] !== n) old.insertBefore(n, old.childNodes[i] ?? null);
  });
}

/**
 * Makes `old`'s children those of `fresh`, keeping each old child equal to
 * the fresh one in its place and taking the fresh node for the rest. For a
 * list of rendered turns, whose nodes carry listeners of their own. A
 * child that `within` picks, in both, is kept and patched the same way.
 */
export function keepEqualChildren(old: Element, fresh: Element, within?: (el: Element) => boolean): void {
  const wanted = [...fresh.childNodes];
  wanted.forEach((f, i) => {
    const o = old.childNodes[i];
    if (!o) old.append(f);
    else if (within && o instanceof Element && f instanceof Element && o.tagName === f.tagName && within(o) && within(f)) keepEqualChildren(o, f, within);
    else if (!o.isEqualNode(f)) old.replaceChild(f, o);
  });
  while (old.childNodes.length > wanted.length) old.lastChild!.remove();
}
