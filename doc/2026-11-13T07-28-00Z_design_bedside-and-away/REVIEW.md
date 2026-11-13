# Review and publication gates

This intake is not an implementation PR. Keep #9359 open and keep the private prototype operating as-is until separately authorized changes are reviewed. Publish child implementation issues after the applicable component designs and evidence are reviewed, not from unreviewed placeholder text.

| Gate | Receipt required |
| --- | --- |
| Source integrity | Complete source register with stable identifiers; statements classified as accepted requirement, option, question, superseded, or deferred; no lost bottom-of-message clauses. |
| Prototype census | Feature-to-source matrix distinguishes working, smoke-tested, manual, local draft, absent, and unverified. Inventory existing files and endpoints, without capturing credentials or production data. |
| Component review | Each packet includes design alternatives, boundaries, contracts, threat cases, rollback, and acceptance oracles. Reviewers cite source IDs, identify unhandled cases, and record dispositions. |
| Implementation mapping | Each child issue traces to a reviewed design and source statements, has dependencies and negative tests, and distinguishes design completion from shipped behavior. |
| Operational handoff | Separately verify deployment owner, scope, version, read/write smoke and rollback. No Archer dispatch until the concrete destination and approval boundaries are resolved. |

## Intake review prompts

1. A shared store is invite-only; an open store is visible only to signed-in site visitors. Does the final model require any other audiences, and what explicit override (if any) is allowed to defeat inherited deny?
2. What can a steering request safely do immediately (for example, reordering), and what should require human review before applying a proposed diff? How should Archer return a result, and where should it land?
3. If a variant is split, keep the original item ID, lineage and current location independent. Resolve empty groups, nested destinations, and what association metadata hidden items expose.
4. Does report grouping use allocation across tags, primary category, or both views? Which refreshes may affect shared defaults versus local views?
5. Distinguish authorization to read a marketplace page from permission to store a specific photo, expose data to another shopper, purchase an item, or send content to an external agent.

## Known draft limits

The local steering draft is manual/export-only and uses the prototype's generic event API. It does not provide server-enforced state transitions, authenticated job delivery, a priority-ordering contract, or verified completion. The local variant draft preserves IDs, supports splitting/grouping, and rejects cycles; association pruning, empty-group handling, nested destinations and ACL-safe projections are design questions. These are useful fixtures, not features in the live release.
