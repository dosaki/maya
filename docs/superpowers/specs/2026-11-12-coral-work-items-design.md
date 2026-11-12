# Work Items: the tab's new name, and Coral as a source

## Intent

Two changes, shipped together as 0.55.0 (a `feat`: minor bump from 0.54.0).

1. **The tab is "Work Items".** The tab that lists issues is called
   "Issues", while its settings are Settings → Work Items. They should
   have one name.
2. **Coral Boards is a Work Items source.** The user wants Coral work items
   "to appear just like the rest of the GitHub/GitLab issues": listed by
   rules in the same tab, with a Start that assigns the item, sets its
   status and opens a session in the right repository.

## Goals

- Every label a person reads says "Work Items" (or "work item") where it
  names the tab: the tab button, the empty-pane text, the Settings help
  lines, the README.
- A Coral source in Settings → Work Items, beside GitHub, GitLab and the
  custom tools, with its own connection (address and token), rules, and
  start status.
- Coral work items and defects show in the tab the same way GitHub and
  GitLab issues do, and Start works on them the same way.

## Non-goals

- No change to how GitHub, GitLab or custom tools behave.
- Projects, phases and subtasks never show (the user's call).
- No Coral writes other than Start's assignment and status, as for the
  forges.
- No Coral comments, reviewers or testers, or creating items.
- No phone or voice work: neither has Work Items today (the phone has
  Sessions, Chat, Debug and Settings; the voice has no work-item
  commands), and this change adds none.

## Anti-goals

- The Coral token must never be written to `config.json`, a log line or
  an error shown to the user. It is stored as the Git tokens are.
- Maya's repository is public: no Coral address of any company is written
  into the code, the tests or the docs. The address is the user's to type.
- Start must never run in the user's own checkouts (as today).

## Constraints

### The tab rename

- The internal tab id `issues` (in `src/tabs.ts`, the `issues-count`
  element, saved tab state) stays as it is, so a saved tab still opens.
- Code comments and type names may keep "issue" where they mean a forge
  issue; only text a person reads changes. Comments that name "the Issues
  tab" change to "the Work Items tab" while the files are open.
- The session name `issue <id>` and the branch prefix `issue-` stay: a
  started item's link to its session depends on them, and earlier started
  items must still match.

### Coral's API

Facts from the Coral Boards source (`packages/web/src/routes/api/v1`,
`packages/api/src`):

- REST under `<address>/api/v1`. A personal token (`cbp_…`) goes in the
  **`x-api-token` header**. Coral reads a `Bearer` token as a Keycloak
  token, so a `cbp_` token sent as `Authorization: Bearer` gets a 401
  (`auth.ts:78-84`). The token acts as the user with the user's
  permissions, has no scopes and no expiry. No rate limit applies to it.
- `GET /api/v1/users/me` → `{id (UUID), email, name, …}`.
- `GET /api/v1/work-items` lists items. Filters: `itemType` (one of
  `phase`, `work_item`, `subtask`, `defect`; one value per call),
  `assignees` (CSV of user ids, assignee role only; there is no "me"),
  `statusCategory=todo,active` (open work), `productCode` (CSV; the call
  404s with "Product not found" only when none of the codes exists),
  `location` (`product`, `triage` or `unassigned`), `tagId` with
  `tagCombinator=and`,
  `page`/`pageSize` (max 100) or `cursor`/`nextCursor`. Projects are not
  in this list. Reply `{data, total, nextCursor?, …}`. Each item has `id`
  (UUID), `displayCode` (`BED-0021`), `title`, `description`,
  `status{id,name,category}`, `assignments[{userId, role,
  user{name,email,…}}]`, `tags[{id,name,scope}]`, `productId`,
  `createdByUser`, `updatedAt`. `product` is always `null` in list and
  detail replies (`enrich.ts:350`).
- `GET /api/v1/work-items/{code}` reads one item (301 to the new code when
  the item moved to another product).
- `PUT /api/v1/work-items/{code}/assignments` with
  `{"assignments":[{"userId","role"}]}` **replaces** the whole list (all
  roles). Several assignees are allowed.
- `PATCH /api/v1/work-items/{code}` with `{"statusId"}` sets the status.
  Coral refuses (400) moving a work item or defect from a "todo" status to
  an "active" one while it has no assignee, and refuses a defect any
  status whose `isAlsoSubStatus` is false (`status-transitions.ts:652`).
- `GET /api/v1/statuses` → one global list
  `[{id,name,category: todo|active|done, isAlsoSubStatus, position, …}]`.
- `GET /api/v1/products/{code}/repositories` →
  `[{url, owner, repo, …}]`. `owner`/`repo` are set for GitHub URLs only.
- An item's web page is `<address>/items/<displayCode>`.
- Tags show in Coral as "scope: name" (`tags.ts:97`), so two tags can
  share a name. Tags and users can be listed (`/api/v1/tags`,
  `/api/v1/users`); the builder confirms their filters and shape against
  the source.

### Maya's code today

- `forge::Http` always sends `Authorization: Bearer <token>`
  (`core/src/forge/mod.rs:262,289,305`). Coral needs `x-api-token`, so the
  trait (or the client) gains a way to send the token in a named header,
  and the fakes follow.
- `read_fresh`, the poll's re-read of started items and the forges' "me"
  assume a repository path, an issue number and a numeric user id
  (`src-tauri/src/work_items_app.rs:62-66,363`). Coral items have none of
  these; each place gets a Coral branch.
- An item with no repo and `tracker` set always gets the repo-pick dialog
  (`needs_pick`, `work_item_check`). For "one repository, no dialog" (see
  Start), `StartCheck` carries the repository Maya found, and the page
  skips the dialog when it has one and no takeover question.

## Approaches

- **A. A built-in Coral source (picked).** A Coral module in
  `core/src/work_items/` next to `github.rs` and `gitlab.rs`, its own
  settings, and its own card in Settings → Work Items. This is what "just
  like GitHub/GitLab" asks for: rules, statuses and Start set up in
  Settings, no script. It is the larger change, about the size of the
  GitLab source.
- **B. A ready-made custom tool.** Maya ships a Coral list script and an
  on-start script for the custom-tool path. Much less code, but it needs
  `curl` and `jq`, keeps the token in a script, and gives no rules or
  status picker in Settings. It feels like a script, not a peer.
- **C. A general "tracker" layer** that GitHub, GitLab and Coral all sit
  behind. Not worth it for one more source; A follows the pattern the
  forges already use.

The user picked A.

## Design

### Settings

`WorkItemsConfig` gets a `coral` section (absent in an older file: off,
with the defaults below):

- `enabled`: off by default.
- `baseUrl`: the Coral address, e.g. `https://coral.example.com`. Required
  when on. Settings refuses an address that is not `https://` (or
  `http://localhost`/`127.0.0.1` for testing), or that has an `@` before
  its path (a user part, which would send the token to the host after
  it), and strips a trailing `/`.
- `token`: a `TokenSource`, as the Git connections have: a token stored in
  the system key store (pasted in Settings) or the name of an environment
  variable. Stored under its own key-store entry, never in `config.json`.
- `rules`: the same `ItemRule` shape as the forges, read for Coral:
  - **Assignee**: me, a named person (by email; Maya looks up the id), or
    anyone.
  - **Tags** (the forges' "labels"): an item must carry all of them.
  - **Products** (the forges' "repositories"): product codes, e.g. `BED`;
    empty means any product.
  - As for the forges, a rule for "anyone" must name its products;
    Settings refuses it and says why.
  - Default: one rule, assignee me, any product.
- `startStatus`: the id of the Coral status Start sets, or none. The card
  shows it by name.

No status labels and no tracker-only list: Coral has real statuses, and
every Coral item finds its repository through its product (below).

### The Settings card

A "Coral" source in the Work Items Sources group, folding like the
others, with its on/off visible when folded:

- **Address**, **Token** (paste, or "From variable" with the variable's
  name, as Settings → Git does), and a **Test** button that calls
  `users/me` and shows "Connected as \<name>" or the reason it failed.
- **Work items to pick up**: the rules box, with "Tags" and "Products" in
  place of "Labels" and "Repositories".
- **On Start, set status to**: a list of Coral's statuses (from
  `GET /statuses`, read when the card opens with a working connection),
  plus "Leave as it is". With no working connection it shows the saved
  status's name, or is disabled with a line saying to test the
  connection first. A help line says that a defect gets the first
  allowed active status when this one is not allowed for defects.
- A foot line: "The token acts as you in Coral: Start assigns the item to
  you and sets its status."

It follows the Settings redesign's shared builders (`src/settings-ui.ts`)
and the vertical layout.

### Listing

- Each poll with Coral on: `users/me` (kept per settings generation, as
  the forges keep "me"), then, per rule, one list call for
  `itemType=work_item` and one for `itemType=defect`, following
  `nextCursor` up to the item cap the custom tools have (500 in all), with
  `statusCategory=todo,active`, `location=product` (only items that live
  in a product: a triage or unassigned one has no page and no product for
  Start to find a repository in), `assignees=<id>` (me or the named
  person; none for anyone), `productCode`, and `tagId`s with
  `tagCombinator=and`. Phases, subtasks and projects are never asked for.
  `users/me` is asked in Coral's own thread of the poll, so a Coral that
  hangs holds up no other source.
- Tag names and named people are resolved to ids per poll (or cached per
  settings generation). *My call:* a rule's tag may be written
  "scope: name" or a bare name; a bare name matches every tag of that
  name, and the rule then matches an item carrying any one of them (one
  list call per combination is not worth it: the rule asks once per
  matching tag and merges). A name Coral does not know, and a rule whose
  product codes Coral answers "Product not found" for (none of them
  exists), is a source error line ("Coral has no tag \<name>", "Coral has
  no product \<code>"), and that rule is skipped; the other rules still
  run. A rule that fails while others answer is a line too, prefixed
  "Coral: ".
- An item becomes a `WorkItem`:
  - `source` `"coral"`, `key` `coral:<item UUID>` (*my call:* the UUID,
    not the code, so a started item stays linked to its entry when it
    moves to another product and gets a new code), `id` the
    `displayCode`, `number` none.
  - `title`, `description` (empty when null; cut at 20,000 characters, as
    the custom tools' are), `url`
    `<address>/items/<displayCode>`.
  - `status` the status name; `labels` the tag names, as Coral shows them
    ("scope: name" for a scoped tag).
  - `assignees` the names of the `assignee`-role users (email when a name
    is missing); `author` the creator's name.
  - `updatedAt` as given.
  - `repo` none and `tracker` true: Start finds the repository (below).
- Items from several rules merge as today (one per key, newest first).
- Errors read like the forges': a refused token (401) says "Coral refused
  the token: paste a new one in Settings → Work Items"; a 403 is a token
  that lacks a permission, not a refused one, so Coral's own message shows
  ("403: Forbidden: items:write permission required"); an unreachable
  address says so.
- A started Coral item that no rule returns any more is read by itself
  (`GET /work-items/{code}`, following a 301) as the forges' started items
  are; it is closed when its status category is `done` or Coral says the
  item is gone (its own 404, "Work item not found"; any other 404, a
  proxy's say, is a failure and the entry stays). It is not re-read in a
  poll where `users/me` failed. The re-read item replaces the started
  entry's copy (new code, title, status), under the same key.

### Start

As for the forges, in the same steps (`work_item_check`,
`work_item_start`):

0. **Takeover**: a fresh read of the item; if someone other than the user
   holds the `assignee` role, the same question names them.
1. **Repository**: the product code is the part of `displayCode` before
   its last `-` (`BED` in `BED-0021`), since replies leave `product`
   out. Maya reads that product's repositories
   (`GET /products/{code}/repositories`). Each one's `url` is matched to a
   forge Maya knows (github.com, or the GitLab connection's host) and
   becomes an `ItemRepo`; others are dropped. Exactly one: Start uses it
   with no dialog, as a plain repo does. None or several: the repo-pick
   dialog shows, as it does for tracker-only repositories, with the
   product's repositories offered first.
2. **Write back**, in this order because Coral needs an assignee before an
   item goes active:
   1. Assign: read the item's assignments, add the user with role
      `assignee` (kept beside any others; reviewers and testers kept), and
      `PUT` the whole list. Skipped when the user is already an assignee.
   2. Status: `PATCH` the start status, when one is set and the item is
      not in it already. *My call:* for a defect, when the start status is
      not allowed for defects (`isAlsoSubStatus` false), Maya sets the
      first status by `position` in the start status's category that is
      allowed; with none, it leaves the status and says so in the notice.
   A failure is a notice, as for the forges; Coral's own 400 message is
   shown (for example the severity a bug needs).
3. and 4. as today: worktree on `issue-<code slug>-<title slug>` (e.g.
   `issue-bed-0021-fix-the-thing`), and a session named `issue BED-0021`.

### Release

- `sh scripts/set-version.sh 0.55.0`, and a `0.55.0` section in
  `RELEASE_NOTES.md` (New: the tab is now Work Items; Coral work items and
  defects show beside GitHub and GitLab issues, and Start works on them).
- README: the Work Items lines say how to connect Coral (address and a
  personal token from Coral).

## What's left to the builder

- Where the key-store entry for the Coral token is named, following the
  Git tokens' naming.
- The exact `tags` and `users` endpoints and filters for resolving names
  to ids, and whether to cache them per poll or per settings generation.
- How the `Http` trait sends a token in a named header, and how the fakes
  follow.
- How the rules editor shares code between the forges' and Coral's
  wording.
- Tests follow the forges': parsing replies, the calls a rule makes, take
  (assign then status, the defect fallback) with a fake `Http`, refusal
  messages, and the Settings card in the DOM tests.
