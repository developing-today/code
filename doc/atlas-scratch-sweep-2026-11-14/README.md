# Atlas scratch sweep — 2026-11-14 (#5981)

Design/reference documents copied from the sandbox workspace's untracked
scratch locations into the tracked repo, per #5981 (Atlas documentation
sweep). **Copy, never delete:** the sources remain in
`~/workspace/scratch/` on the sandbox; a pointer note there names this
directory. Working artifacts (scripts, logs, pickles, JSON state,
git-state snapshots) were deliberately not copied — listed at the bottom.

## Copied documents

| File                                                      | Source (sandbox)                                        | Date       | Subject                                                                                          |
| --------------------------------------------------------- | ------------------------------------------------------- | ---------- | ------------------------------------------------------------------------------------------------ |
| `atlas-ingestion-brief-2026-11-04/prior-art-research.md`  | `~/workspace/scratch/atlas-ingestion-brief-2026-11-04/` | 2026-11-04 | Atlas ingestion program — prior-art & technical grounding (the research other Atlas issues cite) |
| `atlas-ingestion-brief-2026-11-04/statement-inventory.md` | same                                                    | 2026-11-04 | Atlas Ingestion Brief — statement inventory                                                      |
| `atlas-ingestion-brief-2026-11-04/wave2-map.md`           | same                                                    | 2026-11-04 | Atlas Ingestion Brief — wave-2 disposition map                                                   |
| `atlas-restore/SHARDMAP.md`                               | `~/workspace/scratch/atlas-restore/`                    | 2026-11-04 | Atlas core-tier shard restore — working notes                                                    |
| `atlas-restore/comment-5669.md`                           | same                                                    | 2026-11-04 | Core-tier shard restore — executed per Drew's stop-and-redirect ruling                           |
| `atlas-restore/comment-hp20.md`                           | same                                                    | 2026-11-04 | Shard restore executed — follow-through on the spike verdict                                     |
| `atlas-restore/comment-hp21.md`                           | same                                                    | 2026-11-04 | Standing rule: creating Turso databases (Drew ruling)                                            |
| `atlas-scaler-revision/design.md`                         | `~/workspace/scratch/atlas-scaler-revision/`            | 2026-11    | Atlas / Turso data architecture — Scaler-era revision                                            |
| `atlas-spike/FINDINGS.md`                                 | `~/workspace/scratch/atlas-spike/`                      | 2026-11-04 | Atlas shadow-cost spike — findings                                                               |
| `atlas-jaccard-issue.md`                                  | `~/workspace/scratch/`                                  | 2026-11    | Issue draft: Jaccard as a third signal (cf. #5733)                                               |
| `atlas-splade-issue.md`                                   | `~/workspace/scratch/`                                  | 2026-11    | Issue draft: SPLADE investigation (cf. #5741)                                                    |

## Not copied (left at source, by name)

- `atlas-restore/`: `map_build.py`, `create_shards.py`, `load_shards.py`, `load_content.py`, `resume_content.py`, `verify.py` (working scripts), `shardmap.json`, `load_results.json`, `assignments.pkl` (run state/data), `load.log`, `content.log`, `verify.log`, `resume.log`, `resume2.log` (run logs), `__pycache__/` (bytecode).
- `atlas-scaler-revision/`: `ts.txt`, `path.txt`, `head.txt`, `tree.txt`, `blob.txt`, `branch.txt` (git-state snapshots — ephemeral state, not documents).
- `atlas-spike/`: 10 `.py`, 5 `.log`, 3 `.sh`, 1 `.sql`, `assignments.pkl`, `__pycache__/` (spike code and run artifacts; its findings are `FINDINGS.md`, copied above).
- `~/workspace/research_notes/`: no Atlas-titled files found (checked by name, 2026-11-14).
