# Coral rules pick statuses

## Why

A Coral rule lists every open work item and defect, so items already in a
review status show in Work Items. They are implemented; the user never
starts them. The user wants a rule to list only the statuses they start
from.

## Goals

- Each Coral rule can name the statuses it lists, picked from Coral's open
  statuses (the "to do" and "in progress" kinds).
- A rule with no statuses lists every open status, as today, so existing
  rules keep working unchanged.

## Non-goals

- GitHub and GitLab rules: their statuses are labels, which a rule already
  filters by.
- A list of statuses to leave out. The rule names what it includes.
- Done statuses. Maya treats a done item as finished (it leaves the list
  and a started one closes), so they are not offered.

## Approaches

- **A, statuses on each rule (picked).** A Statuses field in each Coral
  rule's box, next to Tags and Products. This is where the user looked for
  it, and rules can differ. With several rules, the same boxes are ticked
  on each.
- **B, one Coral-wide setting.** Set once, but no rule can differ.
- Filtering after the listing was also ruled out: review items would still
  count towards the 500-item cap.

## Design

- **Setting.** A Coral rule saves a list of Coral status ids. An empty list
  is every open status. Older settings without the list read as empty.
- **Settings card.** In a Coral rule, a "Statuses" row of checkboxes, one per
  open status of Coral's, in Coral's order. They are the same statuses the
  "On Start, set status to" list uses, loaded after Test connects. Before
  they load, the row says "Test the connection to list Coral's statuses."
  A saved status that is not in the list (yet) stays ticked, shown as
  "Saved status", so a save does not drop it. A tick or untick saves, as
  the other rule fields do.
- **Listing.** Maya asks Coral only for the rule's statuses (Coral's
  `statusId` filter), still within the open ones. A rule whose statuses
  Coral no longer has gets a warning line, like a missing tag or product,
  for example "Coral has no open status <id>".

## Left to the builder

- Whether Coral's `statusId` takes a comma-separated list, as `productCode`
  does, or needs one request per status. Check against Coral, and merge the
  results the way tag combinations are merged.
- How a rule's warning names a status it cannot find (by id, or by its name
  when Maya still knows it).
- Version and release notes: this is a `feat`, so 0.56.0 from main's 0.55.0.
