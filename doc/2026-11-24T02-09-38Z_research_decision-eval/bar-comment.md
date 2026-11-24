## Part 3b — Luna-as-decision experiment: BAR POSTED BEFORE ANY RUN (Worker C)

No provider has been called yet. This comment states the eval design and the exact pass bar; the run happens only after this post, and the verdict will be judged against the bar exactly as written here.

### Question
Can a harness/prompt wrapper make **Luna low** — chat model `gpt-6-luna` at low reasoning effort, the leg seated as `openai-payg/gpt-6-luna#low` in luna-plan's temporary PAYG tail — meet the Decision API contract (structured yes/no, choice, and rubric-score outputs, schema-faithful), at the level of a native decision leg?

### Eval set (36 items, deterministic expected answers)
Built from the Decision API's documented shapes (Google Gemini Decisions API, `gemini-decision-1`): three families, 12 items each.

- **noul** (yes/no): a state payload plus a yes/no question; several items state an explicit rule/threshold in the state (quota walled below 5% remaining, lane stalled after 15 min without output, merge rule, alert rule) so the expected answer is derivable by rule, not judgement.
- **choice** (pick one of 4 options): support-department routing, queue-row disposition, and free-first leg selection, each with a full option-id → description criteria map.
- **score** (rubric score): severity, evidence-strength, and priority rubrics, 4 ordered levels each (indices 0–3, lowest first), one item per level per rubric.

States are fleet-flavoured (quota windows, lane states, queue rows) plus neutral ones. The full set is saved as `eval-set.json` in the eval artifacts.

### Systems
1. **Luna low + harness** — direct `POST https://api.openai.com/v1/chat/completions`, model `gpt-6-luna`, `reasoning_effort: low`, a decision-engine system prompt plus a strict per-item JSON Schema `response_format` forcing the contract shape `{"value": ..., "confidence": 0..1}` (value: boolean for noul, option-id enum for choice, integer level for score). Keys fetched by name only (`fleet secrets get`), never written to any file or comment.
2. **Baseline: Google Decision 1 native** — direct `POST https://generativelanguage.googleapis.com/v1beta/decisions`, model `gemini-decision-1`, `x-goog-api-key` auth, one request per item, same states/questions in native shape. This also field-proves the endpoint Part 3a is wiring. Jev free is **not** added as a second baseline: no Jev/Zen key is fetchable by name from the secrets broker, and calling it would require routing through the opencode server, which this experiment does not do. Its absence is recorded here, before the run.

### Grading (deterministic, scripted — no vibes)
- **Schema-faithful**: output parses and matches the contract shape exactly (for Luna: exactly the keys `value` + `confidence`, correct value type, value in the allowed set/range, confidence a number in [0,1]; for the baseline: the answer object carries the documented fields for its type).
- **Correct**: decision value equals the expected value. Score-family tolerance: **within ±1 level** counts as correct, for both systems (baseline score value = argmax of its returned probabilities; Luna score value = its integer level).
- Baseline noul value = returned probability ≥ 0.5 → true.

### THE BAR — Luna low + harness PASSES if and only if ALL of these hold
1. Schema-faithful rate **≥ 95%** (at most 1 of 36 outputs non-faithful).
2. Accuracy **≥ 90% overall** (at least 33 of 36 correct).
3. Accuracy **≥ 80% per family** (at least 10 of 12 correct in each of noul, choice, score).
4. Accuracy **no more than 5 percentage points below** the Google Decision 1 native baseline's overall accuracy on the same set.

### Spend bound
Total OpenAI PAYG spend for the whole experiment **≤ $2**, tracked from response usage fields and stated in the verdict. The run stops early if the bound is at risk. Google baseline runs on the free tier.

Artifacts (eval set, harness text, runner, raw outputs, results) are kept with the lane's eval files and summarised in the verdict comment that follows.
