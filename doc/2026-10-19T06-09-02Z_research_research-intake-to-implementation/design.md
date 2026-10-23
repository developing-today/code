# Initial review and research order

[Main reference](main.md) | [Transcript](transcript.txt)

Hard prerequisites use `requires`; other edges express sequencing cost, conflicts, shared files or optional enabling work.

<!-- workflow:view waves -->

Wave 1 can start now: everything it requires is done. Later waves assume the earlier ones finish. Concepts that share files never share a wave. Leases and open branches are live data, so run `ready` before dispatching.

- **Done:** none
- **Running:** none
- **Wave 1:** [C001](main.md#c001)
- **Wave 2:** [C002](main.md#c002), [C003](main.md#c003)
- **Wave 3:** [C004](main.md#c004), [C008](main.md#c008), [C009](main.md#c009), [C013](main.md#c013), [C014](main.md#c014)
- **Wave 4:** [C005](main.md#c005), [C010](main.md#c010)
- **Wave 5:** [C006](main.md#c006), [C011](main.md#c011)
- **Wave 6:** [C007](main.md#c007), [C012](main.md#c012)

<!-- /workflow:view waves -->

## Dependencies

- C002 requires C001: Main reference and design sequencing require a sealed transcript.
- C003 requires C001: Single-writer concept stubs are scaffolded from the sealed transcript.
- C004 requires C003: Preliminary research preparation operates on established concept stubs.
- C005 requires C004: Deep research requires contextual research angles and trigger issues.
- C006 requires C005: Notes reconciliation operates on directed notes generated during deep research.
- C007 requires C006: Issue handoff requires that all cross-concept notes are fully reconciled.
- C008 requires C002: Hierarchical item trails and status evolution belong inside main.md sections.
- C009 requires C003: Worktree boot protocol enforces isolated execution for single-writer concepts.
- C010 requires C009: Thin orchestrator monitors PRs and branches created by the worktree boot protocol.
- C011 requires C005: Disagreement protocol arbitrates conflicting proposals arising from deep research.
- C012 requires C011: Feature planning translates reconciled design records into granular steps.
- C013 requires C002: Cairn library synchronization and docs architecture structure repository references.
- C014 requires C002: Dependency topology governs scheduling across all concept documents.

## Review status

<!-- workflow:view run-status -->

S0 is `validated` when `render` runs, because render only runs on a package that passes `check`. Other stages show the `tasks` entries that record `concept`, `stage` and `status`. `pushed`, `pr-open` and `merged` come from git and GitHub through `status --live`, so they never appear here.

- **C001** [Exact Transcript Storage and Integrity Sealing](main.md#c001): S0 `validated` · S1–S8 `not-started`
- **C002** [Main Reference and Review Order Meta Design](main.md#c002): S0 `validated` · S1 `blocked` (requires C001) · S2–S8 `not-started`
- **C003** [Single-Writer Concept Files and Mutation Invariance](main.md#c003): S0 `validated` · S1 `blocked` (requires C001) · S2–S8 `not-started`
- **C004** [Preliminary Research and Trigger Preparation](main.md#c004): S0 `validated` · S1 `blocked` (requires C003) · S2–S8 `not-started`
- **C005** [Deep Factual Research and Cross-Agent Notes](main.md#c005): S0 `validated` · S1 `blocked` (requires C004) · S2–S8 `not-started`
- **C006** [Asynchronous Notes Reconciliation and Receipts](main.md#c006): S0 `validated` · S1 `blocked` (requires C005) · S2–S8 `not-started`
- **C007** [Hierarchical Issue Handoff and Research Fanout](main.md#c007): S0 `validated` · S1 `blocked` (requires C006) · S2–S8 `not-started`
- **C008** [Hierarchical Items, Status Trails, and Change History](main.md#c008): S0 `validated` · S1 `blocked` (requires C002) · S2–S8 `not-started`
- **C009** [Worktree Boot Protocol and Merge-Commit Pull Requests](main.md#c009): S0 `validated` · S1 `blocked` (requires C003) · S2–S8 `not-started`
- **C010** [Thin Orchestration, Merge Verification, and Worker Lifecycles](main.md#c010): S0 `validated` · S1 `blocked` (requires C009) · S2–S8 `not-started`
- **C011** [Disagreement Protocol, Reviewer Subagents, and Preliminary Decisions](main.md#c011): S0 `validated` · S1 `blocked` (requires C005) · S2–S8 `not-started`
- **C012** [Phased Feature Implementation Planning and Parallel Frontier](main.md#c012): S0 `validated` · S1 `blocked` (requires C011) · S2–S8 `not-started`
- **C013** [Cairn Upstream Library, Generic Reusability, and Docs Architecture](main.md#c013): S0 `validated` · S1 `blocked` (requires C002) · S2–S8 `not-started`
- **C014** [Dependency Topology, Hard Blockers, and Priority Tiers](main.md#c014): S0 `validated` · S1 `blocked` (requires C002) · S2–S8 `not-started`

<!-- /workflow:view run-status -->
