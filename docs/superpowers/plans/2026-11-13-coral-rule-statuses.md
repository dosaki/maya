# Coral Rules Pick Statuses Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Coral rule names the statuses it lists, so items in review statuses stay out of Work Items.

**Architecture:** `ItemRule` gains a `statuses` list of Coral status ids (ignored by GitHub and GitLab rules). The Coral listing passes them to Coral's `statusId` filter in the same request (Coral takes a comma-separated list; checked against Coral on 2026-11-13). The Coral card's rule boxes show a checkbox per open status, from the statuses the card already loads for "On Start, set status to".

**Tech Stack:** Rust (`maya-core`), TypeScript UI with vitest.

**Spec:** `docs/superpowers/specs/2026-11-13-coral-rule-statuses-design.md`

## Global Constraints

- An empty `statuses` list is every open status: today's behaviour, and what older settings read as.
- Offered statuses are the open ones only: Coral `category` `todo` or `active`, never `done`.
- Listing keeps `statusCategory=todo,active` and adds `statusId=<id>,<id>` only when the rule names statuses.
- GitHub and GitLab rules show no Statuses row and their saved JSON does not change (`statuses` is left out when empty).
- No company Coral address, token or email anywhere, tests included: use `coral.test` and made-up ids.
- Settings UI is vertical: the checkboxes stack, one per line (see the "Maya UI: vertical, not side by side" preference).
- Repaint with `morph`/`keepEqualChildren`, never `replaceChildren`, where a node the pointer may be over is replaced (CLAUDE.md lore).
- Release: `sh scripts/set-version.sh 0.56.0`, committed alone with RELEASE_NOTES.md as `chore(release): 0.56.0`.

## Review Focus

1. A saved status Coral has since deleted (or moved to done): the rule lists nothing and says so in a warning, rather than listing every open status. Pinned in Task 1.
2. The Settings page opened before Test has connected: saved statuses stay ticked and survive a save of another field. Pinned in Task 2.
3. Several rules naming statuses: Coral's statuses are asked once per listing, not once per rule. Pinned in Task 1.
4. A Coral rule added with "+ Add rule": it starts with no statuses and saves `statuses: []`, not `undefined`. Pinned in Task 2.
5. Statuses loaded after the card rendered (Test pressed, or the card opened): the rule boxes fill in without a reload, and keep what was ticked. Pinned in Task 2.

---

### Task 1: The rule's statuses, saved and asked for

**Files:**
- Modify: `core/src/config.rs` (`ItemRule`, the Coral rules' normalising near `clean_all(&mut r.labels, ...)` for `w.coral.rules`, tests)
- Modify: `core/src/work_items/coral.rs` (`Lookups`, `list_rule`, `list_type`, tests)
- Modify: every `ItemRule { .. }` literal in `core/src` (add `statuses: vec![]`)

**Interfaces:**
- Produces: `ItemRule.statuses: Vec<String>` serialised as `statuses` (camelCase), `#[serde(default, skip_serializing_if = "Vec::is_empty")]`.
- Produces (private to coral.rs): `Lookups::status_filter(&mut self, ids: &[String]) -> Result<Option<String>, Skip>`: `None` for no ids; else the ids joined by `,` (each `encode`d) when every one is an open Coral status; else `Skip::Warn(format!("Coral has no open status {id}"))` for the first that is not. Coral's statuses are fetched once per `Lookups` (cached like `tags`) with the existing `statuses()`; a failure is `Skip::Fail`.
- `list_type` gains a `status_ids: Option<&str>` parameter and appends `&statusId=<it>` when `Some`.

- [ ] **Step 1: Write the failing tests**

In `config.rs` tests:

```rust
#[test]
fn item_rule_statuses_default_to_empty_and_stay_out_of_the_json() {
    let r: ItemRule = serde_json::from_str(r#"{"assignee":{"kind":"me"},"labels":[],"repos":[]}"#).unwrap();
    assert!(r.statuses.is_empty());
    assert!(!serde_json::to_string(&r).unwrap().contains("statuses"));
    let r = ItemRule { statuses: vec!["s-1".into()], ..r };
    assert!(serde_json::to_string(&r).unwrap().contains(r#""statuses":["s-1"]"#));
}

#[test]
fn normalize_trims_a_coral_rules_statuses() {
    let mut c = Config::default();
    c.work_items.coral.rules.push(ItemRule { assignee: Assignee::Me, labels: vec![], repos: vec![], statuses: vec![" s-1 ".into(), " ".into(), "s-1".into()] });
    // call the same normaliser the existing `normalize_trims_the_coral_address` test calls
    assert_eq!(c.work_items.coral.rules.last().unwrap().statuses, vec!["s-1".to_string()]);
}
```

In `coral.rs` tests (statuses JSON: an array of `{id, name, category, isAlsoSubStatus, position}`; `s-new` todo, `s-prog` active, `s-review` active, `s-done` done):

```rust
#[test]
fn a_rule_asks_only_for_its_statuses() {
    // FakeHttp: ("/statuses", 200, statuses), ("/work-items", 200, page(vec![], None))
    // rule(Assignee::Me, &[], &[]) with statuses ["s-new", "s-prog"]
    // every /work-items url contains "statusId=s-new,s-prog" and "statusCategory=todo,active"
}

#[test]
fn a_rule_with_no_statuses_asks_coral_for_none() {
    // no url contains "/statuses" or "statusId"
}

#[test]
fn a_rule_naming_a_status_coral_has_not_open_warns_and_lists_nothing() {
    // two rules: one with ["s-gone"], one with ["s-done"]; both warn,
    // warnings == ["Coral has no open status s-gone", "Coral has no open status s-done"],
    // no /work-items call is made, and list returns Ok (warnings only, as a missing tag does)
}

#[test]
fn coral_statuses_are_asked_once_per_listing() {
    // two rules with statuses: exactly one url contains "/statuses"
}
```

Check how `list` reports a run where every rule only warned (see `list`'s doc and the existing missing-tag test) and assert the same shape in the third test.

- [ ] **Step 2: Run the tests and see them fail**

Run: `cd core && cargo test --lib -q statuses`
Expected: compile errors (no field `statuses`).

- [ ] **Step 3: Implement**

Add the field; add `statuses: vec![]` to every existing literal; trim and drop blanks and repeats in the Coral rules' normalising with `clean_all`, like `labels`. In `list_rule`, compute `l.status_filter(&rule.statuses)?` once, before the tag combinations, and pass it to every `list_type` call.

- [ ] **Step 4: Run the tests and see them pass**

Run: `cd core && cargo test -q` and `cd src-tauri && cargo test -q`
Expected: all pass. `cargo clippy --all-targets -q` adds no warning in the files touched.

- [ ] **Step 5: Commit**

`feat(work-items): a Coral rule asks Coral only for the statuses it names`

### Task 2: The Statuses checkboxes in a Coral rule

**Files:**
- Modify: `src/types.ts` (`ItemRule`)
- Modify: `src/settings.ts` (`RuleWords`/`CORAL_RULE_WORDS` if needed, `rulesEditor`, `ruleSummary`, the Coral card near `fillStart`/`loadStatuses`/`onAdd`)
- Modify: `src/styles.css` only if the checkboxes need a class of their own
- Test: `src/settings.test.ts`

**Interfaces:**
- Consumes: Task 1's `statuses` JSON field; `CoralStatus { id, name, category, sub, position }` from `src/types.ts`; `model.coralStatuses`.
- Produces: `ItemRule.statuses?: string[]`.
- `rulesEditor` opts gain `statuses?: CoralStatus[] | null`. `undefined` means no Statuses row (GitHub, GitLab). `null` means not loaded yet. Its return gains `showStatuses(list: CoralStatus[]): void`, which refills every rule's row and keeps what is ticked.
- `ruleSummary(rule: ItemRule, statusName?: (id: string) => string): string` adds `, status ${orList(names)}` when `rule.statuses` is non-empty. `statusName` falls back to `"Saved status"`.

The row, in each Coral rule box, after Products: a label "Statuses" and a group (`role="group"`, `aria-label="Statuses"`) with one checkbox per open status (category `todo` or `active`), in the list's order. Each checkbox has `name="wi-statuses"`, `value=<id>` and the status name as its label text. A ticked id not in the list gets its own checked checkbox labelled "Saved status". When the list is `null`, the group holds only those saved ones, and a help line reads exactly "Test the connection to list Coral's statuses." A change saves (`opts.onChange()`). `readRule` returns `statuses` (the ticked values) only for a rule with the row.

- [ ] **Step 1: Write the failing tests**

```ts
it("offers a Coral rule each open status, ticks the saved ones and saves a change", () => {
  // model with coralStatuses [New todo, In Progress active, Review active, Done done],
  // coral rule statuses ["s-new"]
  // checkboxes in the rule: values ["s-new", "s-prog", "s-review"] (no "s-done"), "s-new" checked
  // tick "s-prog" → onWorkItems("coral", items) with items.rules[0].statuses == ["s-new", "s-prog"]
});

it("keeps a Coral rule's saved statuses before Coral's are listed", () => {
  // coralStatuses null, rule statuses ["s-new"]
  // help text "Test the connection to list Coral's statuses."
  // one checked checkbox "Saved status"; changing the rule's Tags saves statuses ["s-new"]
});

it("fills the Coral rules' statuses when they are listed after the card is shown", async () => {
  // coralStatuses null, h.onCoralStatuses resolves the list; open the card the way the
  // existing "lists the statuses after it connects" test does; after it settles, the rule shows
  // one checkbox per open status, "s-new" still checked
});

it("starts a new Coral rule with no statuses", () => {
  // "+ Add rule" in the Coral card → saved rule has statuses: []
});

it("shows no Statuses in a GitHub or GitLab rule", () => {
  // no input[name=wi-statuses] in the GitHub and GitLab cards
});

it("says a rule's statuses in its summary", () => {
  expect(ruleSummary({ assignee: { kind: "me" }, labels: [], repos: [], statuses: ["s-new", "s-x"] }, (id) => (id === "s-new" ? "New" : "Saved status")))
    .toBe("Assigned to me, status New or Saved status");
  expect(ruleSummary({ assignee: { kind: "me" }, labels: [], repos: [] })).toBe("Assigned to me");
});
```

Use the exact `orList` output; check its wording for two items before fixing the expected string.

- [ ] **Step 2: Run the tests and see them fail**

Run: `pnpm -s vitest run src/settings.test.ts`
Expected: the new tests fail.

- [ ] **Step 3: Implement** as the Interfaces block says. The Coral card passes `statuses: model.coralStatuses ?? null` and a `statusName` built from it, and calls `editor.showStatuses(list)` beside `fillStart` when `onCoralStatuses` resolves. `onAdd` pushes `statuses: []`.

- [ ] **Step 4: Run the tests and see them pass**

Run: `pnpm -s vitest run` and `npx tsc --noEmit`
Expected: all pass, tsc clean.

- [ ] **Step 5: Commit**

`feat(work-items): pick a Coral rule's statuses in Settings`

### Task 3: Docs and release

**Files:**
- Modify: `README.md` (the Coral paragraph: rules filter by tags, products and statuses)
- Modify: version files via `sh scripts/set-version.sh 0.56.0`; `RELEASE_NOTES.md`

- [ ] **Step 1:** In README.md's Coral paragraph, after "filter by tags and products", say a rule can also pick the statuses it lists (after Test connects), and that a rule with none lists every open status.
- [ ] **Step 2:** Commit `docs(work-items): Coral rules pick statuses`.
- [ ] **Step 3:** Check main's version is still 0.55.0 (`git show origin/main:package.json`), then run `sh scripts/set-version.sh 0.56.0`, and add at the top of RELEASE_NOTES.md:

```
# 0.56.0

## New

- A Coral rule can pick the statuses it lists in Settings → Work Items, so items already in review stay out of Work Items. A rule with none picked lists every open status, as before.
```

- [ ] **Step 4:** Commit both as `chore(release): 0.56.0`.
