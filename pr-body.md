Closes #15867

Reconciles Group 2 of `doc/atlas-sources-inventory.md` to an all-visibility scope, per lane 2022-E's second-source check on PR #15763 and a fresh live recount in this lane:

- `developing-today`: 31 public + 16 private = **47** total (`gh api orgs/developing-today` reports `public_repos` 31 / `total_private_repos` 16; paginated `orgs/developing-today/repos` recount also returns 47). The merged doc counted this org public-only as 31 while the personal leg used `visibility=all`.
- Four-owner estate census: 47 + 38 + 3 + 546 = **634**, not 618.
- #6023 comparison reworked: its 615 (2026-11-16, three owners) equals today's three-owner count only on the public-only scope for `developing-today` (31 + 38 + 546); the all-visibility three-owner count is 631, so 615 is no longer claimed as directly consistent with an all-visibility recount.
- Recount method now specifies `orgs/<org>/repos?type=all` so the private leg is not silently dropped again.

Verification: remote dispatch on the pushed branch asserting the reconciled counts and the absence of the stale 618/public-only claims (see issue note).
