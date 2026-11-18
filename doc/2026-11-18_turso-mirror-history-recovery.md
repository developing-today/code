# Historical mirror recovery supplement

On November 18, 2026, the reviewed insert-only recovery restored 3,595
archived history records to their canonical `fleet-mirror` Turso database:

| Target table                          | Recovered rows |
| ------------------------------------- | -------------: |
| `agent_mirror_history_agent_agents`   |            941 |
| `agent_mirror_history_goals_goals`    |            299 |
| `agent_mirror_history_goals_updates`  |              1 |
| `agent_mirror_history_scheduler_jobs` |          2,354 |

The first three targets were absent. The scheduler history target already
existed but was empty, with its canonical nonunique lookup index. No
existing record was replaced, updated or deleted. All pre-existing schema
entries were verified unchanged inside the transaction. The table DDL is
from the existing `pkgs/agent-backup/turso_client.py` contract. This recovery
creates missing tables, not additional optional lookup indexes.

PR #11946 was independently reviewed before production execution. Its
fixed four-table candidate SHA-256 is
`9d8a50601d2ec42afb5b9c69ca80c7e66bdddb52151c19cae0f4fe05248001b9`.
The full candidate was rehearsed on a dedicated Turso stage. A fresh
production preflight passed; one transaction inserted and verified all
rows before COMMIT. Fresh-connection readback then verified exact sorted
row-multiset hashes, preserving original TEXT values, NULLs and duplicate
multiplicity. A second, separate read-only verification completed at
**2026-11-18 22:19:04 UTC** with the same four counts and hashes.

Private recovery evidence files are `mirror-history-stage-verification.json`,
`mirror-history-production-verification.json` and
`mirror-history-independent-readback.json` in the local recovery directory.
These receipts contain counts and hashes, not payloads or credentials.
Thirty-five importer tests passed after integrating current main, including
atomic rollback, nonempty-target refusal, malformed response, baton loss
and redirect refusal checks. No scheduler or producer was restarted by this
history import.

## Queue archive boundary

The separate read-only queue key audit found 538 recovered snapshot keys
absent from current operational targets: 5 tasks, 15 lanes, 34 workers,
389 lane-ledger records, 89 events, 3 repository snapshots and 3 watermarks.
Those records remain in the complete recovery archive. They were not
replayed into the newer live queue: old queued, enabled or working states
could resurrect obsolete work. Key presence is not full row-value parity.
The private audit receipt is `queue-key-coverage.json`. These intentional
archive-only records are distinct from the completed mirror-history import.
