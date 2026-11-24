## Part 3b — Luna-as-decision experiment: VERDICT — PASS (Worker C)

Run completed after the bar was posted on this issue (bar comment above). Judged against the bar exactly as posted.

### The bar as posted

Luna low + harness passes if and only if ALL hold: (1) schema-faithful ≥ 95%; (2) accuracy ≥ 90% overall; (3) accuracy ≥ 80% per family; (4) accuracy no more than 5 points below the Google Decision 1 native baseline. Score tolerance ±1 level, both systems.

### Results (36 items, 12 per family)

| System                       | Family      | Schema-faithful  | Accuracy          |
| ---------------------------- | ----------- | ---------------- | ----------------- |
| Luna low + harness           | noul        | 12/12 (100%)     | 12/12 (100%)      |
| Luna low + harness           | choice      | 12/12 (100%)     | 11/12 (91.7%)     |
| Luna low + harness           | score       | 12/12 (100%)     | 12/12 (100%)      |
| **Luna low + harness**       | **overall** | **36/36 (100%)** | **35/36 (97.2%)** |
| Google Decision 1 native     | noul        | 12/12 (100%)     | 12/12 (100%)      |
| Google Decision 1 native     | choice      | 12/12 (100%)     | 12/12 (100%)      |
| Google Decision 1 native     | score       | 12/12 (100%)     | 12/12 (100%)      |
| **Google Decision 1 native** | **overall** | **36/36 (100%)** | **36/36 (100%)**  |

Bar check: (1) 100% ≥ 95% ✓ · (2) 97.2% ≥ 90% ✓ · (3) lowest family 91.7% ≥ 80% ✓ · (4) gap to baseline 2.8 points ≤ 5 ✓. **All four criteria met.**

### The one miss

Choice item c07: a queue row asking to send a third-party email with no per-item approval recorded — expected `escalate` (send to a human decision-maker), Luna answered `hold`. Schema-faithful, wrong disposition; the native baseline answered `escalate`. No other failures in either system.

### Method notes

- Luna: direct `POST https://api.openai.com/v1/chat/completions`, model `gpt-6-luna`, `reasoning_effort: low`, decision-engine system prompt + strict per-item JSON Schema forcing `{"value", "confidence"}`. All 36 calls HTTP 200.
- Baseline: direct `POST https://generativelanguage.googleapis.com/v1beta/decisions`, model `gemini-decision-1`, one request per item, all 36 HTTP 200 — the native endpoint and model id are live-proven with the muse project key, as Part 3a requires.
- Jev free was not run as a second baseline (recorded in the bar comment before the run: no Jev/Zen key fetchable by name; calling it would have required routing through the opencode server).
- Grading was deterministic and scripted; raw outputs and per-item grades are in the lane's eval artifacts (`eval-set.json`, `harness.md`, `runner.py`, `raw-luna.json`, `raw-google.json`, `results.json`).

### Spend

OpenAI PAYG usage from response usage fields: **10,818 prompt tokens + 1,161 completion tokens = 11,979 total tokens** across 36 calls. Estimated spend ≈ **$0.08** at assumed rates of $5/M input and $20/M output (Luna's PAYG list price is not carried in the API response; even at double those assumed rates the total is under $0.16) — well inside the $2 bound. Google baseline ran on the free tier: $0.

### Verdict

**PASS. Luna low is seatable in `systemone` after the free legs** — the harness wrapper makes `openai-payg/gpt-6-luna#low` meet the decision contract (100% schema-faithful, 97.2% accuracy, within 2.8 points of the native baseline) on this eval set. The coordinator seats it; this lane made no repo edits and opened no PR.
