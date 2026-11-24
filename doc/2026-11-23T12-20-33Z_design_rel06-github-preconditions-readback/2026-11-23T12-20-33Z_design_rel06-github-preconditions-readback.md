# REL-06 — Effect-specific GitHub preconditions and read-back verification (#9516)

- **Slice:** REL-06 of the #9209 component (leases, fencing, transactional
  outbox & direct verified completion), pm-orchestration program #9196.
- **Issue:** #9516, `design(pm-orchestration): Effect-specific GitHub
preconditions and readback adapter`.
- **Status:** design record. This is the bounded design deliverable for
  #9516. It is a V3 proposal in the issue's own terms: it does **not**
  authorize implementation. Implementation starts only under a
  separately scoped implementation issue, after this record is merged
  and the granular prerequisites in §11 are resolved.
- **Design parent:** #9209, landed by PR #9225 as
  `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`
  (cited below as "the #9209 package"). The REL-06 slice spec is
  `components/9209-leases-fencing-outbox-direct-completion.md` §11;
  the governing decision is `decisions/FDR-PMO-006-outbox-direct-completion.md`.
- **Owned file (implementation, future):**
  `pkgs/fleet/crates/fleet/src/gh.rs` plus its inline adapter contract
  tests. No new client crate, no new queue, database, scheduler,
  service, resolver, or executable.
- **Explicit read-only consumer:**
  `pkgs/fleet/crates/fleet-queue/src/work/operations.rs` (REL-03's
  idempotency model). This slice reads its receipt/ledger contract;
  it does not edit it.
- **Pinned baseline of the source slice:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.
  Current-state evidence below was re-verified against origin/main
  `7fecf882f614cfd6642b8a387009489a19a354ec` on 2026-11-23.

## 1. Purpose

The outbox design (#9209, FDR-PMO-006) commits Fleet state, journal,
and an immutable operation intent in one local transaction, then
delivers the operation's remote effects to GitHub. GitHub shares no
transaction with Fleet, does not accept Fleet fence tokens, and
several of its writes are asynchronous or non-idempotent. The piece
that keeps delivery honest is the **effect adapter**: for each kind
of GitHub effect it

1. checks an **effect-specific precondition** immediately before the
   send (so a stale intent cannot act on a changed remote state),
2. performs the send through the existing `fleet gh` surface, and
3. **reads the remote state back** and returns a verdict — verified,
   pending, unknown, or conflict — instead of treating the send's
   HTTP response as proof the effect happened.

REL-06 is that adapter's contract. It covers the GitHub effects the
direct-completion protocol needs first (PR merge, issue close,
comment create) plus the read verbs that serve as its read-back
channels, and it registers the one effect family that already exists
(the #9073 metadata-stamped create path) instead of redesigning it.

Out of scope, owned elsewhere and consumed here only as contracts:
operation identity and receipt storage (REL-03 #9513, REL-04 #9514),
the relay that drives sends (REL-05), the completion state machine
that consumes verdicts (REL-07), CLI registration (CLI-13 / #9206),
field authority and mirror policy (AUTH-01 #9429, AUTH-02 #9499,
#9208), schema ownership for any persisted field (CEN-04 #9434).
Nothing here re-decides those slices; where their contracts are
still open, this record names the assumption it makes and the gate
that must confirm it (§11).

## 2. Current state, verified (done check)

No REL-06 adapter exists on origin/main today:

- `pkgs/fleet/crates/fleet/src/gh.rs` (2,734 lines) is a passthrough:
  every argument reaches the real `gh` unchanged behind a token
  rotation, and the caller sees the final attempt's stdout, stderr,
  and exit code byte-exact. Its write handling is a **retry-safety
  classification** (#8887 contract): a write is repeated under a
  fresh token only when the failure proves GitHub refused the
  request before processing it (rate limit, permission/scope
  rejection); any other write failure is returned as-is because the
  request may have landed. There is no precondition check, no
  merge-specific handling (`pr merge` is merely classified
  not-read), and no read-back verification anywhere in the file.
- Two effect-specific interceptions already exist in the same file
  and are the precedents this design generalizes:
  - the **close guard** (#9177, `gh_closure.rs` / `gh_guard.rs`):
    `issue close` and `issue edit --add-label status:done` are
    refused (exit 2, before the rotation runs) when the queue broker
    holds a claimed/parked task for the item, unless a
    `fleet-closure/v1` completion record is presented via the
    wrapper-only flag `--completion-record <path>`, which is
    stripped before `gh` runs;
  - the **metadata create path** (#9073 slice 9, `gh_metadata.rs`):
    `issue create` / `pr create` bodies are parsed fail-closed,
    stamped with the §5 provenance set from fleet sources only,
    signed when a key is configured, and inserted into the metadata
    store after a successful create; a failed insert leaves the item
    intact and is healed by ingestion (#12590).
- `fleet-queue/src/work/contracts.rs` already carries the shared
  envelopes the adapter's output must fit: `fleet.command/v1`
  (`operation_id`, `idempotency_key`, `request_digest` as a
  `sha256:` fingerprint, `expected_work_version`) and
  `fleet.result/v1`, whose per-effect record is
  `Effect { field, requested, observed, verified }` — requested,
  observed, and verified states kept separate. REL-06's receipts
  extend that shape; they do not invent a parallel one.
- `fleet-queue/src/work/completion.rs` already carries REL-09's
  freshness policy as a pure module (its header cites #9519). The
  family is landing slice by slice; REL-06 is the missing
  GitHub-side piece between REL-04's outbox and REL-07's completion
  state machine.

## 3. Settled inputs consumed, not re-decided

| Input              | What REL-06 takes from it                                                                                                                                                                                                                                                       |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #9209 component §4 | Operation model: immutable intent, expected remote precondition carried on the operation, per-target ordering, retry classes (terminal / conflict / unknown-after-send / rate-limit backoff), supersede rules.                                                                  |
| #9209 component §5 | Direct completion phases; merge with expected head SHA; 202 is accepted/enqueued, never merged; per-method merge verification (§5.4); issue closure verified per issue; partial effects stay recoverable.                                                                       |
| #9209 component §6 | Comment/body writes have no remote CAS: GET-compare is not optimistic concurrency; the design must not promise prevention of a human race.                                                                                                                                      |
| #9209 component §8 | Failure-matrix rows for stale head, async merge, squash/rebase results, partial close, duplicate delivery.                                                                                                                                                                      |
| FDR-PMO-006        | Local transactional intent + effect-specific adapter + remote observation/reconciliation + direct local finalization (option 1, adopted). "Effectively once" is scoped, never universal exactly-once. GitHub close/readback may not prove actor attribution; report confidence. |
| REL-03 (#9513)     | Operation id + idempotency key + request digest; the stable structured receipt is shared by commands. Same key + same digest replays the receipt; same key + different digest is a deterministic conflict.                                                                      |
| REL-04 (#9514)     | The outbox intent is committed atomically with local state + journal; the adapter is a delivery/verification step over that intent, never a second store of intent.                                                                                                             |
| AUTH-01 (#9429)    | Every write checks authenticated principal capability and scope, expected version, and reason. The adapter refuses an effect the presented operation is not authorized for; it does not itself grant scope.                                                                     |
| AUTH-02 (#9499)    | Public mirroring is explicit, allowlisted, operation-scoped, default private. A comment effect exists only because an operation names it; the adapter never mirrors on its own initiative.                                                                                      |
| CEN-04 (#9434)     | One owner per persisted fact. REL-06 persists nothing new (§9); any future persisted receipt field goes through CEN-04's matrix first.                                                                                                                                          |
| #9177 close guard  | Precedent for refusing before send inside `gh.rs`, and for a wrapper-only flag (stripped before `gh` runs) as the inter-component channel.                                                                                                                                      |
| #9073 create path  | Precedent for effect-specific interception in `gh.rs` (fail-closed parse → stamp → send → record), and the first registered effect family (§5).                                                                                                                                 |

## 4. Adapter shape

### 4.1 Position

The adapter lives in `gh.rs`, in the `fleet` crate, beside the two
existing interceptions. It is an internal contract, not a user CLI:
the callers are the REL-05 relay and the REL-07 completion state
machine, which hand it one **effect intent** at a time, derived from
a committed outbox operation. It reuses the existing machinery
rather than adding a GitHub client:

- sends and read-backs run through the same token rotation and the
  same failure classification as every `fleet gh` call, so identity,
  failover, and the #8887 retry-safety rule are inherited, not
  reimplemented;
- because the rotation can change which identity a send used, all
  verification is **state-based, never actor-based** (§7).

### 4.2 Invocation seam

Two entry modes, mirroring the existing precedents:

1. **Programmatic (primary).** Relay/completion code calls the
   adapter with an `EffectIntent` (§6) and receives an
   `EffectReceipt` (§8). This is the path the outbox uses.
2. **Operation-scoped interception.** When a `fleet gh` invocation
   carries an operation context — presented exactly the way the
   close guard's record is: a wrapper-only flag naming a local
   intent file, stripped before `gh` runs — the matching effect
   family is routed through the adapter instead of the plain
   passthrough, and the receipt is written back to the operation's
   store by the caller. The flag is an inter-component channel, not
   registered user syntax; per the CLI-06 → REL-06 facade
   acceptance, the command inventory must mark the exact spelling.

A plain `fleet gh` call with no operation context is **unchanged**:
byte-exact passthrough, existing failover, existing guard and
create-path behavior. The adapter is capability-gated by the
presence of an intent, which is also the rollback switch (§10).

### 4.3 Test seam

Contract tests need a fake remote, not the network. The adapter
sends through a narrow seam over the existing run machinery (the
same pattern as the existing `TokenProvider` seam in
`run_with_provider`): production binds it to the real rotation;
tests bind it to a scripted fake that records an exact request
ledger and can return 200, 202, 403, 404, 409, and
apply-then-timeout (the send is applied to the fake's state, then
the response is dropped). All verdict logic is a function of the
intent plus the fake's responses and state, so the named oracles
(§9) run hermetically. Tests on the fake establish adapter logic
only; live GitHub behavior remains separately reported, per the
family rule.

## 5. Effect catalogue

The commands allocated to REL-06 (issue, command allocation) sort
into four treatments. Read verbs are not effects; they are the
read-back channels, and the adapter's verifications are expressed
in terms of them.

| Treatment                                    | Allocated commands                                                                                                                                                                                                                                                                                                     | Adapter behavior                                                                                                                                                                                      |
| -------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Preconditioned, read-back-verified write** | `github.pr.merge`, `github.issue.close`                                                                                                                                                                                                                                                                                | Full contract: §6 preconditions, §7 verification. These are the effects direct completion depends on.                                                                                                 |
| **Marker-verified write**                    | `github.comment.add`                                                                                                                                                                                                                                                                                                   | Send carries the stable operation marker (REL-03 identity); verification adopts by marker (§7.3).                                                                                                     |
| **Compare-verified write**                   | `github.comment.edit`, `github.issue.update`, `github.pr.update`                                                                                                                                                                                                                                                       | GET-compare against the intent's baseline digest before send; read-back compares the managed content only. No CAS is claimed (#9209 §6); a baseline mismatch is `conflict`, never a silent overwrite. |
| **Registered existing write**                | `github.issue.create`, `github.pr.create`, `fleet-gh.create`                                                                                                                                                                                                                                                           | Already implemented by the #9073 metadata create path in `gh.rs`. REL-06 registers it as an effect family whose receipt is the create outcome (created URL + metadata record status); no redesign.    |
| **Read-back channels (reads)**               | `github.pr.get`, `github.pr.checks`, `github.pr.commits`, `github.pr.diff`, `github.pr.reviews`, `github.issue.get`, `github.issue.comments`, `github.issue.timeline`, `github.comment.list`, `github.comment.reply` (a write shaped as a reply; treated as `comment.add` when under an intent, passthrough otherwise) | Plain passthrough outside an intent; inside verification, the adapter issues them itself and treats their results as evidence, not as caller-supplied claims.                                         |
| **Passthrough, unchanged**                   | `fleet-gh.passthrough`, `flag.*` spellings, `agent.slash.merge`, `flag.close-issues`, `flag.close-linked-issues`, `flag.merge-pr`, `flag.merge-linked-prs`, `flag.comment-id`, `flag.issue`, `flag.pr`, `flag.verify`                                                                                                  | Flag/spelling inventory is preserved verbatim (compatibility rule); the flags select intents for the layers above the adapter. The adapter defines no spelling and normalizes none.                   |
| **Consumers of receipts**                    | `queue.stack.replay`, `sync.outbox.retry`, `sync.verify`, `work.complete`                                                                                                                                                                                                                                              | Not adapter entry points: they consume the verdicts in §8 (retry only on the classes §6.4 permits; verify = run the read-back again; complete only on verified required effects, decided by REL-07).  |

## 6. Preconditions

An `EffectIntent` carries, from the committed operation: operation
id, idempotency key, request digest (REL-03), the effect kind, the
canonical target ref (`owner/repo#number` plus kind), the declared
parameters (merge method, close reason, marker), the precondition
below, and the caller's deadline. The adapter re-reads the target
immediately before the send; a precondition is evaluated against
that fresh read, never against the state the intent was authored
from.

### 6.1 `pr.merge`

- **Expected head SHA — required.** The intent names the PR head SHA
  the completion was authorized against. The send is the REST merge
  call with the expected SHA as its precondition parameter (the
  `gh pr merge` CLI exposes no SHA precondition, so under an intent
  the adapter issues the API call through `fleet gh api` machinery,
  not the CLI verb). A changed head fails the precondition before
  or at the send: verdict `conflict` (`REMOTE_CONFLICT` family),
  recovery = rehydrate and re-authorize. The adapter never retries
  with the new head on its own.
- **PR state.** Fresh read must show the PR open and not already
  merged. Already merged → skip the send, go straight to
  verification (§7.1) — this is what makes a re-presented intent
  safe after a lost acknowledgement. Closed and not merged →
  `terminal_failure` for this effect (§9, closed-not-merged).
- **Method is declared, never inferred.** Merge, squash, or rebase
  comes from the intent; the receipt records it, because
  verification semantics differ per method (§7.1).
- **Checks, reviews, branch policy** are GitHub's own gate at the
  send and REL-07/#9207's refresh responsibility before it; the
  adapter does not reimplement branch protection. A policy refusal
  from the API (not mergeable / protected) is `terminal_failure`
  pending operator, not a retry loop.

### 6.2 `issue.close`

- **Explicit designated ref only.** The intent names the exact
  issue. The adapter never closes linked, parent, epic, or
  component issues by inference (#9209 §5, §8).
- **Target really is an issue.** The fresh read must not carry a
  `pull_request` object; a PR number presented as an issue close is
  refused as ambiguous before any send.
- **Expected state open, reason explicit.** Close reason
  (`completed` / `not_planned`) is declared in the intent. Already
  closed → verification only (§7.2), no resend.
- The #9177 close guard continues to run first and unchanged: an
  adapter close is still refused when a live bound task exists
  without its completion record.

### 6.3 `comment.add`

- **Stable operation marker — required for verifiability.** The
  body carries the operation's marker (the REL-03 operation
  identity in the family's established marker form). An intent
  without a marker is accepted only as an _unverifiable_ effect:
  the receipt can never rise above `unknown` after a lost
  acknowledgement, and completion policy must not count it as a
  verified required effect. This is stated, not hidden.
- **Mirror scope.** The intent exists only under AUTH-02's
  allowlist and consent rules; the adapter performs no mirroring
  decision of its own.

### 6.4 Send and retry discipline (all effects)

Inherited from #9209 §4 and the #8887 rule already in `gh.rs`:

- Failure classes that prove refusal **before** execution
  (rate limit, permission/scope rejection) may fail over or retry
  per provider guidance and the caller's deadline.
- **Revoked permission** observed at the pre-send revalidation or
  as a send refusal stops future sends for the operation
  (`AUTH_DENIED`, terminal pending operator). Revocation does not
  cancel a request already sent; that request's outcome is resolved
  by read-back.
- A timeout or connection drop **after** send ⇒ the effect is
  `unknown`. The next step is always read-back, never a blind
  resend of a non-idempotent effect.
- No retry budget, poll interval, or ceiling is invented here;
  budgets derive from existing fleet policy, provider
  `retry-after`, and the caller's deadline (#9209 §4).

## 7. Read-back verification

Verification reads the remote state through the §5 read channels
and compares it to the intent. The verdict set is shared by all
effects:

- `verified_applied` — remote state matches the intent and the
  evidence ties it to this operation (SHA precondition held, marker
  found, method receipt consistent).
- `verified_observed` — remote state matches the intent's desired
  end state, but attribution to this operation is unavailable
  (e.g. the issue is closed; who closed it cannot be proven from
  the read). State satisfaction and attribution are reported
  separately, per FDR-PMO-006; whether `verified_observed` counts
  toward completion is REL-07 policy, never an adapter decision.
- `pending` — the remote has accepted but not completed the effect
  (merge queue / async merge). Never reported as success.
- `unknown` — a send may have landed and read-back cannot yet
  confirm or deny it (lost acknowledgement, delayed visibility).
- `conflict` — remote state contradicts the intent (head moved,
  baseline digest mismatch, duplicate markers).
- `refused_precondition` — nothing was sent; the §6 precondition
  failed against the fresh read.
- `terminal_failure` — the remote refused in a way retry cannot
  fix (closed-not-merged PR, revoked permission, invalid target).

### 7.1 Merge verification, per method

After any merge send not proven refused-before-execution, read the
PR back (`github.pr.get`) and decide:

- `merged == false`, PR still open → `pending` if a 202/queued
  acceptance was seen, else `unknown`; re-read within the caller's
  deadline, then leave `pending`/`unknown` to the caller's recovery
  — the adapter does not spin.
- `merged == false`, PR closed → `terminal_failure`
  (closed-not-merged). A closed PR is never a merged PR.
- `merged == true` → verify against the declared method:
  - **merge commit:** the returned merge commit must link the
    expected head — the merge commit's second parent is the
    expected head SHA (checked via the commits read channel), and
    `merge_commit_sha` is recorded on the receipt.
  - **squash:** GitHub rewrites history; the expected head is
    **not** an ancestor of the result and the adapter must not test
    ancestry. Verification is: `merged == true`, the declared
    method on the receipt is squash, and `merge_commit_sha` (the
    squash commit) is recorded. That is the whole proof; the
    receipt says so.
  - **rebase:** likewise, head commits are rewritten. Verification
    is `merged == true` plus the recorded `merge_commit_sha` /
    resulting base state as returned; ancestry of the original
    head is not required.
- A 202 response is recorded on the receipt as acceptance evidence
  and changes nothing about the verdict until a read-back shows
  `merged == true`: **`pmo_rel_merge_202_is_not_verified_complete`**
  is the standing oracle for this rule.
- A 409 from the merge call is the SHA precondition firing at the
  remote: verdict `conflict`, no resend with any other SHA.

### 7.2 Issue-close verification

Read the issue back: `state == closed` and, where the API returns
it, `state_reason` equal to the declared reason. Attribution: the
timeline read channel may identify the closing event; when it
cannot tie the closure to this operation, the verdict is
`verified_observed` with `attribution: unavailable` on the receipt
— the **ambiguous issue closure** case. The adapter never upgrades
an observed closure to `verified_applied` on state alone. A
designated issue still open after a send that was not refused is
`unknown`, resolved by re-read, not by resending the close (a
resend is harmless on GitHub's side but would falsify the attempt
history REL-03 keeps).

### 7.3 Comment verification

Read the target's comments and match the operation marker:
exactly one match → `verified_applied` with the comment id
recorded; zero matches after a send that may have landed →
`unknown` (delayed visibility is re-read under the caller's
deadline before the verdict settles); more than one match →
`conflict` (a duplicate exists; recovery is REL-07/operator
policy, the adapter never deletes). This is the lost-acknowledgement
path of #9209 §5.2: never POST a second comment solely because the
first response timed out.

### 7.4 Create verification (registered, existing)

The #9073 path already defines it: the created item's URL parsed
from the send result, the item read back through the metadata
record, and a failed store insert reported as a healable miss
rather than a failed create. REL-06's receipt for the family is
that outcome, unchanged.

## 8. Receipt shape

The adapter returns receipts; it does not store them. Storage is
REL-04's outbox; identity is REL-03's. The receipt extends the
`contracts.rs` effect record (requested / observed / verified kept
separate) with the fields verification needs:

```text
EffectReceipt
  operation_id, idempotency_key        (from the intent, REL-03)
  effect_kind                          (pr.merge | issue.close |
                                        comment.add | comment.edit |
                                        issue.create | pr.create | …)
  target                               (canonical repo#number + kind)
  precondition                         (expected head SHA /
                                        expected state / baseline
                                        digest, as applicable)
  send                                 (method declared, HTTP status
                                        class, attempted-at, identity
                                        source by name only)
  observed                             (remote state at read-back:
                                        merged flag, merge_commit_sha,
                                        issue state + reason, comment id)
  verdict                              (§7 set)
  attribution                          (verified | unavailable)
  retry_class                          (#9209 §4 class + next action)
```

Serialization follows the shared contract conventions in
`contracts.rs` / REL-03's docs (snake_case, canonical decimal
strings for versions, `sha256:` digests). Error outcomes reuse the
#9209 §7 stable codes (`REMOTE_CONFLICT`, `REMOTE_UNAVAILABLE`,
`AUTH_DENIED`, `IDEMPOTENCY_KEY_REUSED`, …) carried in the
`fleet.result/v1` error envelope. No field in this shape is
persisted by REL-06 itself; if the implementation finds a field
that must be persisted beyond REL-03/REL-04's existing shapes, that
field waits for CEN-04's compatibility matrix (§11).

## 9. Oracles and done check

Owner target: the adapter lives in the `fleet` crate, so the
owning target is that crate's test suite — not the `fleet-queue`
base command the #9209 package quotes generically for its
queue-side slices. The issue's done rule applies unchanged: list
the tests by exact name first (`-- --list`), assert they exist,
then run the filter; a zero-test filter passing is not acceptance.

```sh
cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet --features full -- --list pmo_rel_
cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet --features full pmo_rel_merge_
cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet --features full pmo_rel_
```

Named oracles (the issue's three first, then the family):

- `pmo_rel_merge_rejects_stale_head_sha` — head moved after intent
  authoring ⇒ `refused_precondition`/`conflict`, zero merge sends
  in the fake's ledger.
- `pmo_rel_merge_202_is_not_verified_complete` — fake answers 202
  and stays unmerged ⇒ verdict `pending`, never verified.
- `pmo_rel_merge_method_receipt_handles_squash` — squash merge
  whose result does not contain the head as ancestor ⇒
  `verified_applied` on PR metadata + squash commit SHA, with no
  ancestry assertion.
- `pmo_rel_merge_rejects_closed_not_merged` — read-back shows
  closed, `merged == false` ⇒ `terminal_failure`.
- `pmo_rel_merge_queued_is_pending_not_merged` — merge-queue
  acceptance, read-back open ⇒ `pending`.
- `pmo_rel_merge_apply_then_timeout_reads_back_verified` — fake
  applies the merge, drops the response ⇒ `unknown`, then read-back
  ⇒ `verified_applied`, with exactly one merge send in the ledger.
- `pmo_rel_merge_revoked_permission_sends_nothing_further` — 403
  scope refusal at revalidation ⇒ `AUTH_DENIED`, no retry under a
  second identity for the same effect.
- `pmo_rel_close_readback_distinguishes_ambiguous_closure` —
  issue closed without attributable closing event ⇒
  `verified_observed`, `attribution: unavailable`.
- `pmo_rel_close_refuses_pull_request_target` — target carries a
  `pull_request` object ⇒ refused before send.
- `pmo_rel_comment_timeout_adopts_single_marker_match` — POST
  applied then timed out; exactly one marker comment ⇒
  `verified_applied`, no second POST.
- `pmo_rel_comment_duplicate_marker_is_conflict` — two marker
  matches ⇒ `conflict`.

Negative coverage required by the issue maps one-to-one:
closed-not-merged, queued-not-merged, revoked permission, stale
SHA, ambiguous issue closure, squash ancestry assumption — each is
a named oracle above, run against the fake remote's 200 / 202 /
403 / 404 / 409 / apply-then-timeout repertoire.

## 10. Compatibility, migration, rollback

- **Compatibility.** Plain `fleet gh` behavior is unchanged
  (§4.2); existing spellings — including `expedite` / `next-up`
  priority spellings the family rules protect — are untouched; the
  close guard and metadata create path keep their current behavior
  and precedence (guard first, then adapter, then passthrough).
- **Migration.** None in this slice: no schema, no persisted field,
  no stored-state change. The capability arrives gated by the
  presence of an operation intent.
- **Rollback.** Disable the adapter capability (stop presenting
  intents) and `fleet gh` is exactly the prior passthrough.
  Pending intents and already-issued receipts stay in REL-04's
  store, untouched; remote effects already applied are not undone
  — there is no rollback of a merge, only the record of it.
- **Schedules.** This design creates, changes, and retires no
  schedule on any surface; `doc/fleet-schedule-registry.md` needs
  no entry for it.
- **Secrets.** Identity sources are named, never valued, exactly
  as `gh.rs` does today; receipts record the token _source name_
  only.

## 11. Gates and sequencing for implementation

This record is the design deliverable for #9516. A separately
scoped implementation issue may be filed when:

1. this record is merged;
2. the granular prerequisites are resolved — verified **open** on
   2026-11-23, and consumed here as contracts, not results:
   REL-03 #9513 (receipt identity the adapter returns),
   REL-04 #9514 (the outbox intent it delivers),
   AUTH-01 #9429 and AUTH-02 #9499 (the authorization and mirror
   scope it enforces but does not define),
   CEN-04 #9434 (only if §8 grows a persisted field);
3. the implementation names its fake-remote seam and lands the §9
   oracles fail-first in the `fleet` crate target.

Downstream consumers wait on this slice, not the reverse: REL-05's
relay drives the adapter, REL-07's completion state machine
consumes its verdicts (including its call on whether
`verified_observed` satisfies a required effect), REL-11's recovery
runbook operates its `unknown`/`conflict` outcomes. QTI remains
unlocated/unverified (#9241); nothing here presumes a QTI
equivalence or changes a persisted schema while that stands.

## 12. Requirement trace (allocated IDs → where this record answers them)

Full per-occurrence citations remain in the #9209 package's
`census/requirements-crosswalk.jsonl`; this is the slice-level map.

| IDs                                                                                    | Answered in                                                                                                                                                                                              |
| -------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| C-R005, C-R107, C-R108, H-R001, H-R017, H-R024, H-R031, P-R001, P-R002, P-R009, P-R012 | §4–§5: one adapter inside the existing cohesive `fleet gh` surface; full allocated command/flag inventory preserved and classified, none renamed.                                                        |
| C-R007, C-R143, C-R144, C-R145, C-R146, H-R015, O-R025, O-R034, P-R011, P-R022, P-R033 | §6–§7: the merge → verify → close → verify chain direct completion runs on, with no webhook/sweeper on the normal path (C-R147's webhook-for-recovery role is preserved by the `unknown` recovery path). |
| C-R009, C-R150–C-R153, H-R016, O-R021, O-R049, O-R050, O-R052, P-R027, P-R035, P-R036  | §7, §9: the walkthrough scenarios' remote-effect steps (verify merge/closure on resume, checkpoint-instead-of-unsafe-merge) resolve to adapter verdicts and named oracles.                               |
| C-R087                                                                                 | §5: slash `merge` and the `flag.*` spellings are inventory preserved verbatim; semantics attach at the intent layer, not the spelling.                                                                   |
| C-R110                                                                                 | §4.2: composed workflows (`work next/park/complete`) reach the adapter only through intents; no workflow logic lives in the adapter.                                                                     |
| C-R140, C-R141                                                                         | §6, §8: stable operation id + expected version/SHA stored on the intent and receipt; verify-before-retry is the §6.4 discipline.                                                                         |
| C-R148, O-R035, P-R023, P-R038                                                         | §7, §9: partial effects (merge verified, close failed) stay per-effect receipts in a recoverable state; no aggregate success is reported.                                                                |
| H-R014, O-R027, O-R032, O-R033, P-R007, P-R013, P-R021, P-R025                         | §3, §6.4, §8: fencing stays local (the adapter claims no remote fence); lifecycle, ownership, and GitHub state stay distinct — a merge or a closure alone is never work completion.                      |
| H-R015 (adapter half), O-R022, O-R023, O-R026, O-R029                                  | §4–§5: the GitHub adapter is one responsibility with stated boundaries; hydrator/reconciler/resolver stay other slices' modules.                                                                         |
| P-R040                                                                                 | §6.1, §9: a stale or unverifiable merge is refused, not rushed — the near-timeout path checkpoints instead.                                                                                              |

## 13. What this record deliberately does not decide

- Poll intervals, retry counts, and deadlines (caller policy and
  provider guidance, §6.4).
- Whether `verified_observed` satisfies a required completion
  effect (REL-07 policy).
- Any merge-queue-specific read field beyond the PR read-back in
  §7.1; if the live API exposes a richer queue state at
  implementation time, the implementation issue records it as
  evidence, it does not silently extend this contract.
- GraphQL merge as an alternative send path. The REST merge call
  is the designed path because the SHA-precondition/409 semantics
  are the ones the #9209 evidence pins; a GraphQL variant is a new
  decision, not an implementation detail.
- Hosted parity. The hosted worker doors (#9081 family) reproduce
  queue semantics behind their own interfaces; whether they grow
  this adapter is #9209 V4's question, answered there, not here.
