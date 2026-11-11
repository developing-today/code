# Creation outbox — bounded pilot (#3706)

Status: pilot implementation landed with task 13263 (lane code-y,
2026-11-10). Fleet-wide rollout is NOT authorized: Drew approved the
bounded pilot on 2026-11-04, and wider conversion waits for the live
trials below plus a fresh owner ruling. Design of record: the
task-8938 design delta on #3706 (comment 6623964166).

## What exists

- **Core** (`pkgs/fleet/crates/fleet-queue/src/creation_outbox.rs`):
  the state machine — payload hashing (SHA-256 over canonical JSON),
  dedupe/conflict semantics, exponential backoff with jitter and
  Retry-After precedence, the account circuit breaker, pacing
  constants (2 s minimum gap, 300 creations/hour budget), marker
  format, and metrics (pending depth, oldest pending age, attempts,
  last failure class, circuit state).
- **Storage + broker API** (`turso_queue.rs`): table
  `creation_outbox` in the fleet-queue Turso database (created by
  `ensure_schema`, beside the queue tables) with
  `UNIQUE(repo, dedup_key)`, a due-work index, and the persisted
  circuit row (`creation_outbox_circuit`). Verbs:
  `/outbox-enqueue`, `/outbox-claim`, `/outbox-resolve`,
  `/outbox-list`, `/outbox-metrics`.
- **Client + drainer** (`client.rs`, `fleet queue outbox …`):
  `enqueue | list | metrics | claim | resolve | drain`. The drainer
  is serial: claim the oldest due entry, submit through `gh` with
  the hidden marker `<!-- fleet-outbox:<id> -->` in the body, verify
  by read-back (marker + number), then resolve. A `done` without a
  verified number/URL is refused by the broker.

## Rules the implementation enforces

- Same `(repo, dedup_key)` + same payload returns the original id;
  same key + different payload is a conflict, never an overwrite.
- A `gh` exit 0 never marks done: read-back verification does.
- Ambiguous creates (empty success, lost response) go `uncertain`
  and reconcile by marker before any re-create; an expired lease is
  claimed as `uncertain` for the same reason (a crashed drainer's
  create may have landed).
- 403/429: Retry-After is honored exactly; otherwise exponential
  backoff (30 s base, 1 h cap, jitter). Three consecutive rate
  limits open the circuit for 300 s; no create bypasses it.
- Kinds are allowlisted: `issue`, `pull_request`, `issue_comment`,
  `pr_comment`. Reviews, releases, and interactive human `gh` use
  are out of scope.
- Credentials never enter rows or logs (payloads are the public
  title/body/labels/target only).
- The markdown action-outbox and its processor are unchanged and
  remain the explicit prepared-action fallback.

## Pilot producer

Exactly one producer is wired: **new-issue-creator-code-a**. Its
creation step calls

```bash
fleet queue outbox enqueue --repo <owner/repo> --kind issue \
  --dedup-key <producer>:<stable-intent-key> \
  --producer new-issue-creator-code-a --payload-file <intent.json>
```

instead of creating directly, and a drainer runs
`fleet queue outbox drain --owner outbox-drainer` on the pilot's
cadence. No dual-write: if the enqueue fails, the producer reports
the failure — it does not fall through to direct `gh` creation
(the markdown outbox stays available only as the explicit,
human-visible fallback per its own protocol).

**Activation order matters:** the broker verbs exist only after the
queue-broker is rebuilt/redeployed from a tree containing this
change. The producer flip is therefore a follow-on step after
deploy, tracked as a separate queue task — flipping the cron first
would point it at verbs the live broker does not serve yet.

## Acceptance trials

The four trials from the design run as automated tests against a
fake creator (oracle-named tests in `creation_outbox.rs`,
`cargo test -p fleet-queue creation_outbox`):

1. **50-item burst** — all ids issued, serial drain within budget,
   every object read-back verified, no duplicate, no loss.
2. **Kill/restart** — drainer dropped with a row leased and its
   create landed; restart after lease expiry reconciles that row by
   marker, drains the rest, exactly one object per key.
3. **Forced 403** — with Retry-After (honored exactly; circuit opens
   after repeated 403s; nothing bypasses it; delivery resumes after
   it clears) and without (exponential backoff).
4. **2026-11-01 replay** — a gh-style empty success is never counted
   done without read-back; the marker recovers the landed object;
   when the marker search proves absence, the same entry re-creates
   exactly once.

Plus: dedupe replay/conflict, read-back-failure → uncertain,
backoff growth/cap, metrics depth/age.

**Live-trial follow-on (not run by the pilot PR):** after the
broker deploys, re-run trials 1–3 against a scratch repo with the
real drainer and record the results on #3706, then flip the
creator-a producer. That evidence, not the fake-creator tests, is
what Drew's rollout ruling should cite.
