# Changes

[Main reference](main.md)

- 2026-10-19T06:09:02Z | intake | prepared | Initial transcript and concept scaffold created.
- 2026-10-21T20:04:24Z | S0 | created | C001.1 added under C001: Raw source must remain completely unedited regardless of typos or noisy speech-to-text.
- 2026-10-21T20:04:24Z | S0 | created | C001.2 added under C001: Tooling must refuse non-UTF-8 unless explicitly encoded, and prevent git hooks from altering trailing spaces.
- 2026-10-21T20:04:24Z | S0 | created | C002.1 added under C002: main.md acts as the central link target for all downstream artifacts.
- 2026-10-21T20:04:24Z | S0 | created | C002.2 added under C002: design.md proposes an initial sequencing order for reviewing concepts, separating blockers from parallel paths.
- 2026-10-21T20:04:24Z | S0 | created | C003.1 added under C003: Concurrent edits to shared wide files cause merge conflicts that drop information.
- 2026-10-21T20:04:24Z | S0 | created | C003.2 added under C003: Partitioning writers by file eliminates merge collisions during parallel research.
- 2026-10-21T20:04:24Z | S0 | created | C004.1 added under C004: A lightweight exploration pass positions the research without flooding the context window.
- 2026-10-21T20:04:24Z | S0 | created | C004.2 added under C004: Trigger issues provide zero-context handoff briefs for dedicated research workers.
- 2026-10-21T20:04:24Z | S0 | created | C005.1 added under C005: Research workers operate in parallel on isolated concept documents.
- 2026-10-21T20:04:24Z | S0 | created | C005.2 added under C005: Workers never edit files owned by other agents; they write immutable notes to notes/.
- 2026-10-21T20:04:25Z | S0 | created | C006.1 added under C006: Every note must be explicitly incorporated, acknowledged, declined, or superseded with rationale.
- 2026-10-21T20:04:25Z | S0 | created | C006.2 added under C006: Reconciliation must scan remote branches to catch notes arriving from parallel workers.
- 2026-10-21T20:04:25Z | S0 | created | C007.1 added under C007: Parent issues define the overall goal; child issues isolate granular research facets.
- 2026-10-21T20:04:25Z | S0 | created | C007.2 added under C007: Issues must be self-contained so a zero-context agent can execute them without conversation history.
- 2026-10-21T20:04:25Z | S0 | created | C008.1 added under C008: Information is clarified or superseded rather than erased.
- 2026-10-21T20:04:25Z | S0 | created | C008.2 added under C008: Status changes record timestamp, stage, and rationale with backlinks to refined items.
- 2026-10-21T20:04:25Z | S0 | created | C009.1 added under C009: Pushing the branch immediately reserves the ref on origin.
- 2026-10-21T20:04:25Z | S0 | created | C009.2 added under C009: Merge commits (--no-ff) preserve historical development context and eliminate rebase churn.
- 2026-10-21T20:04:25Z | S0 | created | C010.1 added under C010: Orchestrators must avoid implementation drift; their duties are coordination and merge gating.
- 2026-10-21T20:04:25Z | S0 | created | C010.2 added under C010: Merges are executed only when remote SHA matches PR head matches worker reported SHA.
- 2026-10-21T20:04:25Z | S0 | created | C011.1 added under C011: The pipeline must not halt for human input on routine architectural choices.
- 2026-10-21T20:04:25Z | S0 | created | C011.2 added under C011: Reviewers analyze merged data, adopt standard defaults as agent-preliminary, and log genuine trade-offs.
- 2026-10-21T20:04:25Z | S0 | created | C012.1 added under C012: Plans specify behavior and test contracts rather than full speculative code.
- 2026-10-21T20:04:25Z | S0 | created | C012.2 added under C012: Parallel steps are marked [P] and mapped to non-overlapping worktree partitions.
- 2026-10-21T20:04:26Z | S0 | created | C013.1 added under C013: Generic stage contracts and tools live in Cairn; target repositories keep local adapters in profiles.
- 2026-10-21T20:04:26Z | S0 | created | C013.2 added under C013: Documentation is structured into root entry points, doc/ historical records, and planning/ plans.
- 2026-10-21T20:04:26Z | S0 | created | C014.1 added under C014: Soft sequencing edges document rework risks without artificially blocking parallel workers.
- 2026-10-21T20:04:26Z | S0 | created | C014.2 added under C014: Contingent paths carry explicit activation triggers and are deferred until needed.
