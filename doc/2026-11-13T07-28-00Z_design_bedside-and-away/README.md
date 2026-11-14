# Bedside & Away: design-mill intake

Spec: [#9359](https://github.com/developing-today/code/issues/9359). Status: **design intake for review; no implementation approval**.

The owner asked for a source-complete design mill before the app lands in this repository. A functioning private prototype predates this intake. Keep it available, keep its data in place, and do not mistake its behavior for a ratified decision or a finished implementation. This directory intentionally contains no application code or runtime credentials.

- [Requirement ledger](requirement-ledger.md): stable source references, distinct requirements, prototype coverage and gaps.
- [Architecture and decomposition](architecture.md): component design boundaries, dependency order, review and acceptance gates.
- [Workstream plan](PLAN.md) and [review gates](REVIEW.md).

The first review should reconcile the ledger with the source messages, decide identity and visibility semantics, and agree on boundaries for the external agent trigger. Next, produce one reviewed component design per workstream (split further if needed), then linked implementation issues with prerequisites, source IDs, observable acceptance oracles, and rollback requirements. Do not dispatch Archer/Muse or connect a credential merely to validate this design. No production writes or public sharing are part of this PR.

The live baseline used the separate Sites source revision `57b86e8` for UI/API, plus laptop-staged external runtime adjustments. Local steering and variant drafts are evidence for future design review, not shipped features. Verification before this intake was limited to prototype tests, owner login, live authenticated Turso read/write, and search. The two live entry points do not share a database.

Keep personal access details, account identities, cart links and private conversations out of the repository. Use stable source references without copying identifying data.
