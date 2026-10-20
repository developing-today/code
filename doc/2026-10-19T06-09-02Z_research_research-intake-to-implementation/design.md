# Initial review and research order

[Main reference](main.md) | [Transcript](transcript.txt)

Hard prerequisites use `requires`; other edges express sequencing cost, conflicts, shared files or optional enabling work.

Ready frontier: C001. Run one agent per ready concept with exclusive owned files.

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

Not run. Research, design and feature planning remain separate later stages.
