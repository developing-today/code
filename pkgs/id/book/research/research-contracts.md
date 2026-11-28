# Three design records for repository review

These proposals were drafted November 23, 2026 against the behavior identified at the linked revisions. Each specifies an independent acceptance result and a decision for the owning implementation. Adoption and runtime verification are tracked separately.

## Exact arithmetic in the supply audit

Compare total posted issuer debit with total eligible holder, fee and burn credit under one accounting view. A backend read failure or a total outside the supported range must remain distinguishable from a clean result. A fixture with issuer debits 2^127+1 and a single holder credit of 2^127 has an independent mathematical difference of one. Cover the balanced counterpart and account-order independence. The full record discusses typed report compatibility and how to handle unrepresentable totals. [Full design record](https://github.com/developing-today/code/blob/main/doc/2026-11-23T14-05-00Z_design_id_supply_reconciliation/2026-11-23T14-05-00Z_design_id_supply_reconciliation.md)

## Visibility across search, navigation and replication

Declare which metadata each peer class can learn before comparing data stores. A strict reader should observe the same allowed IDs, aggregate counts, cursor behavior, rank and layout when the stores differ only by protected facts. Fully trusted replication can have wider inventory visibility. The record distinguishes the operator policy from the test fixture and limits the observation set to the surfaces actually promised. [Full design record](https://github.com/developing-today/code/blob/main/doc/2026-11-23T14-05-00Z_design_id_derived_visibility/2026-11-23T14-05-00Z_design_id_derived_visibility.md)

## Recovering authority across interrupted effects

One user command can produce a durable intent, an external effect and separately readable receipts. Model a committed effect followed by a lost response and a grant change. The expected effect count is one. Current policy separately governs obtaining a prior receipt, sending a new request and seeing audit details. A provider without an idempotency or lookup contract reports an uncertain outcome. [Full design record](https://github.com/developing-today/code/blob/main/doc/2026-11-23T14-05-00Z_design_id_retry_effects/2026-11-23T14-05-00Z_design_id_retry_effects.md)
