## OpenRouter post-reset through-route probe — route completed, but NOT served by OpenRouter (2026-11-24)

Owed verification from the KEY-CREATE-2 record, run after the free-tier reset (2026-11-24T00:00Z), through the route on the laptop's opencode server: session create → model-set to `opencode-route/openrouter-free-plan` → prompt "Reply with exactly: OK". Key referenced by name only (BWS `openrouter.api-key`); no values printed anywhere.

**What happened:** the probe client's transport timed out mid-run and the laptop-agent's automatic SSH fallback re-ran the script once, so two identical probe sessions exist. No further attempts were made — the ≤2 attempt budget for this check is spent. Results below are read back from the server's own session records (verified second source, not the lost client output):

| Session                          | Created (UTC)        | Outcome   | Reply | Actually served by                     |
| -------------------------------- | -------------------- | --------- | ----- | -------------------------------------- |
| `ses_e5df517b246ffeJ0gELxdSW3d0` | 2026-11-24T00:15:26Z | succeeded | `OK`  | `claude-payg` / `claude-haiku-5-5`     |
| `ses_e5df515d48effeP9qByXnInFJ3` | 2026-11-24T00:17:28Z | succeeded | `OK`  | `_muse` / `muse-spark-1.3-contributor` |

Both sessions record a model-switched event to `opencode-route/openrouter-free-plan`, the user prompt, an assistant reply of exactly "OK", and outcome `succeeded`. No 429 surfaced to either session.

**Verdict — the owed item is NOT closed in substance:**

- The route accepted and completed both prompts, but **neither completion was served by an OpenRouter :free model**, even though `openrouter-free-plan`'s own block seats only `openrouter/*:free` targets (round-robin, no fallback edges in the plan block). The completions came from providers outside the plan.
- So there is still **no verified OpenRouter-served completion through the route after the reset**, and this probe cannot distinguish "OpenRouter legs still failing (e.g. the account-level 429) and masked by route fallback/escalation" from "OpenRouter legs never attempted". The sessions' route-target records are empty, so the mechanism is not visible in the session record.
- Next diagnostic (not run here — this job's probe budget is spent): one direct, non-route minimal completion against OpenRouter with the vault key, and/or route-attempt logging for a future through-route probe, so the serving leg is attributable.

Recorded 2026-11-24T00:22Z by the `openrouter-route-probe-after-reset` scheduled job.
