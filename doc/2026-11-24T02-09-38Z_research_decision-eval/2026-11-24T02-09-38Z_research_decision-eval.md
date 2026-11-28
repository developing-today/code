# Luna-as-decision experiment — research record

The decision-API model experiment (Worker C, code#13675 Part 3b): can a harness/prompt wrapper make **Luna low** — chat model `gpt-6-luna` at low reasoning effort — meet the Decision API contract (structured yes/no, choice, and rubric-score outputs, schema-faithful) at the level of a native decision leg?

## Verdict (from `verdict-comment.md`)

**PASS.** Luna low + harness is seatable in `systemone` after the free legs: 36/36 schema-faithful (100%), 35/36 correct (97.2% accuracy), within 2.8 points of the native baseline — all four bar criteria met.

| System                              | Schema-faithful | Accuracy      |
| ----------------------------------- | --------------- | ------------- |
| Luna low + harness                  | 36/36 (100%)    | 35/36 (97.2%) |
| Google Decision 1 native (baseline) | 36/36 (100%)    | 36/36 (100%)  |

The one miss: choice item c07 (a queue row asking to send a third-party email with no per-item approval recorded — expected `escalate`, Luna answered `hold`). Spend: ~$0.08 OpenAI PAYG across 36 calls (inside the $2 bound); Google baseline ran on the free tier.

## Provenance

Migrated 2026-11-24 from `hatch-personal/workspace/decision-eval/` under the DOCS-MIGRATION programme (#13877). Source files are copied byte-for-byte, preserving names. This is a dated research record: it preserves what was present in the source set at migration time and is not rewritten later.

## Contents

- `bar-comment.md` — the eval design and the exact pass bar, posted before any run.
- `verdict-comment.md` — the verdict, judged against the bar exactly as posted.
- `harness.md` — the harness text (system prompt, wire format, grading rules).
- `eval-set.json` — the 36-item eval set (12 noul, 12 choice, 12 score).
- `runner.py` — the deterministic grading runner.
- `raw-luna.json` — raw Luna outputs (36 calls, all HTTP 200).
- `raw-google.json` — raw Google Decision 1 native baseline outputs.
- `results.json` — per-item grades.

The migrated source set contains 8 files. Including this front page, the record has 9 files.
