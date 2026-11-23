# Troubleshooting

Symptom → cause → fix. Each entry names the check that confirms the cause before you apply the fix. For the health surfaces referenced throughout, see [Health checks and the heal stack](health-and-healing.md); for how the services are run, see [Running the brokers as services](services.md).

## Session proxy down / "proxy closed during CONNECT"

- **Symptom:** Fleet commands that need the laptop fail with transport errors (exit 3); proxy connections die during the CONNECT handshake.
- **Cause:** One of the first legs of the heal chain is down: tailscaled, the local relay on port 3130, or the session broker itself. The session proxy is how a sandbox seat reaches the laptop's brokers, so its failure looks like every broker failing at once.
- **Confirm:** `fleet agent doctor` — the queue and shell legs are down while the github leg may still answer.
- **Fix:** Let the heal chain rebuild in order (tailscaled → relay :3130 → session broker → secrets broker → gh); it runs on a roughly 5-minute cron, or re-run the seat's init bring-up. Do not start the session broker by hand ahead of the relay — a broker started before its relay answers but reaches nothing.

## Secrets broker stuck refreshing (503, very many `bws` fetches)

- **Symptom:** `fleet secrets get <name>` fails or reports the broker not serving; `GET /health` on the secrets-broker answers 503 with `serving=false` for a long time; the broker's refresh appears to run constantly (the shape recorded as issue #13081 involves on the order of 220 `bws` fetches in a refresh cycle).
- **Cause:** The broker's full-vault refresh has not yet *succeeded*. `serving` only becomes true once a refresh completes (`last_refresh_unix` is set); a refresh that keeps failing or being restarted keeps the broker in the not-serving state, and `/health` reports `refresh_error` when a refresh has failed.
- **Confirm:** `GET /health` — check `refreshing`, `last_refresh_unix`, and `refresh_error`. A cached read that succeeds carries `source: "cache"` and an `age_seconds`; that tells you the broker once served and how stale the value is.
- **Fix:** Fix the refresh's cause (the `bws` route and its token, by name — never by pasting a value anywhere), then let **one** refresh complete. **Do not restart-loop the broker:** every restart discards refresh progress and restarts the clock, converting a slow recovery into a permanent 503. A process that is alive and returning 503-with-`serving=false` is in a known state, not crashed.

## Queue double-serve: a parked, completed, or cancelled task is served again (#13091)

- **Symptom:** A task whose issue record shows it parked, completed, or cancelled is handed out by `fleet queue claim` as if it were live work.
- **Cause:** The broker-side state and the GitHub issue record have diverged; the broker re-served a task whose terminal or parked disposition lives in the issue record. Recorded as issue #13091.
- **Confirm:** Read the task (`fleet queue show <id>`) and the GitHub issue. Treat **the issue record as the source of truth** for disposition.
- **Fix (workaround):** Re-park the task (`fleet queue park <id>`) so the broker state matches the record, and note the double-serve on the issue. Do not work the task a second time, and do not complete it "again" — a duplicate completion corrupts the lane ledger. The no-double-serve guard (the client checking a fresh claim against the GitHub fallback and holding it with reason-class `fallback_held`, exit 5) covers the claim path it can see; a re-serve that bypasses it is the bug, not operator error.

## Stub: exit 69

- **Symptom:** A command prints a three-line block to stderr and exits 69, without doing any work.
- **Cause:** The domain is not usable in this invocation. Exit 69 is `EX_UNAVAILABLE`: the stub is the binary working as designed, not a crash. The first line decodes which of the four wordings you got:
  1. *"this build does not include '&lt;domain&gt;'"* — **NotInBuild**: the capability was never compiled into this variant (for example, asking `fleet-connect` for the queue).
  2. *"invoked as '&lt;name&gt;', this build does not include '&lt;domain&gt;'"* — **NarrowedByName**: the binary has the domain, but the hyphenated argv0 profile narrowed it out. Re-invoke as plain `fleet`.
  3. *"'&lt;domain&gt;' is disabled by config on this host"* — **DisabledByConfig**: the `[domains]` table in this host's `fleet.toml` disables it. The stub names the config file; re-enable it there if the disable is not a deliberate incident lever.
  4. *"'&lt;domain&gt;' is included in this build but not implemented yet"* — **Available-form**: compiled in, nothing behind it.
- **Confirm:** `fleet capabilities` — the domain's `state` field says the same thing in JSON (`not-in-build`, `narrowed-by-name`, `disabled-by-config`, `available`), and the second stub line lists the capabilities this invocation *can* use.
- **Fix:** Match the wording: use the full build, invoke as `fleet`, or re-enable in config. Never "fix" a stub by reinstalling — the install is telling you the truth about itself.

## GitHub rate limits: user token versus the App route

- **Symptom:** `gh` calls (direct or via `fleet gh`) fail with rate-limit errors — `rate limit`, `rate_limit`, `too many requests`, HTTP 429.
- **Cause:** The token that made the call exhausted its bucket. The user (personal) token and the GitHub App installation tokens are separate buckets with separate limits.
- **How `fleet gh` handles it:** `fleet gh` rotates credentials in a fixed order — the personal token first, then the fleet App's installations, then the developing.today App's, then the backup App's — and a rate-limit (or permission) failure is one of the two classes that may be retried under the next token, because the limiter refuses the request *before* it is processed, so replaying a write is safe. Other failure classes are retried under a fresh token only for read-only calls.
- **Fix:** Prefer `fleet gh` over plain `gh` for fleet work so rotation applies. If you hit a limit on plain `gh`, switch routes rather than retrying in a loop — a rate-limited write that cannot go through the CLI route belongs in the creation outbox, which paces creates and backs off on consecutive rate limits (300 s, up to its attempt budget) instead of hammering. Check which route you are on with `fleet agent doctor` (the github leg reports `route: "fleet"` or `"plain"`).

## Claim rejected: lane/worker mismatch (#6853 shape)

- **Symptom:** Holder verbs (`heartbeat`, `checkpoint`, `park`, `release`, `block`, `fail`, `complete`) fail with exit 5 and a "no matching claimed row" error, although the task is visibly claimed.
- **Cause:** The broker checks the holder by worker **and lane**: all scheduled lanes share one worker id, so the lane is what tells the broker whose claim a verb acts on. A verb issued with the wrong `--lane` (or a lane from a different registered lane/worker family — the `registered_lane_worker` families of issue #6853) does not match the claimed row.
- **Confirm:** Compare the claim's lane and worker (`fleet queue show <id>`, the claim output) with what your command resolved: `--lane` flag, else the `REPO_QUEUE_LANE` environment variable, else null (broker checks the worker alone). Also confirm the row is non-terminal — the same exit 5 covers "already terminal".
- **Fix:** Re-issue with the claiming lane (`--lane <id>`) and the claiming worker. Do not release and re-claim to work around it; that churns the lease and the timeline.

## TLS `UnknownIssuer` from sandbox seats (#13122)

- **Symptom:** HTTPS calls from a sandbox seat fail certificate validation with an `UnknownIssuer` error, while `gh` itself works.
- **Cause:** The sandbox's egress proxy intercepts TLS. A client validating against the webpki root store alone does not trust the interception chain; the platform root store does. (This is why `fleet gh`'s App legs validate against the platform root store rather than webpki roots alone.)
- **Confirm:** The same call succeeds through `gh` or through a platform-store client, and fails only from the webpki-only client. Recorded as issue #13122.
- **Fix:** Use the route that validates against the platform store (`fleet gh` for GitHub API work). Do not disable certificate verification anywhere as a workaround — an unverified channel for fleet credentials is worse than the outage.

## Lease expiry mid-work

- **Symptom:** Work you are holding is claimed by someone else, or your next `heartbeat` / `complete` fails with exit 5 ("no matching claimed row").
- **Cause:** A claim is a lease, not a lock. Every claim first runs lease expiry opportunistically: abandoned claims whose lease lapsed become claimable at the moment anyone looks. If your heartbeats stopped (a stalled session, a long silent build), the lease lapsed and the task returned to the pool.
- **Confirm:** `fleet queue timeline <id>` shows the expiry and any re-claim. Check the lease length you claimed with against the gap in your heartbeats.
- **Fix:** Prevention, not recovery: heartbeat on a cadence well inside the lease, and checkpoint before long silent operations. If the task was re-claimed, coordinate through the issue record before touching it again — two workers on one task is the failure this system exists to prevent. If nobody re-claimed it, claim it again and continue.

## `verify-disposition` gate rejections at `complete`

- **Symptom:** `fleet queue complete` rejects the completion and **leaves the task claimed**, with a message naming a defect in the disposition record.
- **Cause:** Completion of a fix-class task requires a verified disposition: the readback record produced by `fleet queue verify-disposition <issue> --repo <owner/name>`, which reads the issue and its linked PRs back from GitHub live. The broker's gate is pure (no network) and checks: the record's freshness (≤ 1800 s old by default; more than 300 s in the future is unreadable clock skew), that it describes *this* task's issue, that the verdict matches the readback (for example `merged_verified` requires at least one merged pull in the readback), that every PR the completion links appears in the readback, and any follow-up (a follow-up issue must be open, not the issue being completed, and must link the completed issue and every unfinished PR; a successor PR must be in the readback and merged; a follow-up task is checked broker-side for a live status naming the same issue).
- **Confirm:** Run the verb standalone — `fleet queue verify-disposition <issue> --repo <repo> --json` exits 0 when its record would pass the gate and 1 when the gate would reject it, so you can iterate without touching the claim.
- **Fix:** Address the named defect — most often a stale record (re-run `verify-disposition` and complete promptly), a verdict the readback does not support (the work is not actually in the state claimed; finish or follow up the work first), or a missing follow-up for the `open_draft_with_followup` / `open_draft_superseded` verdicts (record the follow-up with `--reason`, which any follow-up target requires). The gate leaving the task claimed is deliberate: a rejected completion loses no work.
