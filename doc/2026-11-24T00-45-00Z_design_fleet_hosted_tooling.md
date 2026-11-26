# Design: fleet tool for hosted editions (Workers/TypeScript equivalent)

- Date: 2026-11-24 (UTC), lane code-i, queue task 21562
- Source issue: [developing-today/code#9081](https://github.com/developing-today/code/issues/9081)
- Parent epic: #9078 (hosted editions program), tracking #13873, umbrella #13871
- Status: design only. No implementation in this document.
- Related native work: #9073 (fleet gh Metadata create-path stamping and DB insert), #9074 (metadata database and fleet Metadata query), #9075 (session state and provenance), #9016 (fleet gh step 2, higher-level sync tools), #9076 (webhook and sweeper dual ingestion), #9077 (relying-party tokens).

## 1. Problem and context

The native `fleet` binary gives the laptop world a set of guarantees: issues and PRs are created with Metadata stamps (queue id, scheduled start, pickup and work-start times, worktree and host, session id, model, lane-start issue set); Metadata is queryable; session provenance is recorded; and sync tools run on a rotation of GitHub credentials. None of that exists for the hosted editions, which run on Cloudflare Workers with Turso as the database. A hosted agent or user that files an issue today gets no stamp, no provenance and no sync guarantee, so hosted activity is invisible to the same queries the laptop fleet relies on.

Evidence: the source issue's open questions list the same four gaps (split of duties, stamp parity, auth of stamps, queue semantics). The native surface is documented in `pkgs/fleet/docs/verbs.md`. The Workers fit was investigated in `doc/2026-10-28T23-34-14Z_investigation_cloudflare_workers_fit.md`, and the platform-side Workers design is in `doc/2026-10-14T10-18-19Z_design_p7_2_cloudflare_workers.md`.

## 2. Goals and non-goals

Goals:

- G1. A hosted caller can create issues and PRs whose Metadata stamp has the same field names as the native stamp, wherever the field has a meaning in the hosted world.
- G2. Stamps written by the hosted service are queryable with the same filter language as `fleet` Metadata query.
- G3. Session provenance is recorded for hosted sessions in the same record shape as native sessions.
- G4. Stamps cannot be forged by a caller that does not hold the authority to write them.
- G5. A decision on whether the hosted world gets its own queue, and on what that queue's packet schema is.

Non-goals:

- N1. Rebuilding the whole native fleet binary on Workers. Verbs that need a worktree, a shell, a laptop socket or a local build stay native.
- N2. Replacing GitHub as the system of record for issues and PRs.
- N3. Designing the hosted UI.
- N4. Choosing the auth provider. #9077 owns relying-party tokens; this design only names what it needs from them.

## 3. Alternatives considered

A. Do nothing. Hosted activity stays unstamped and invisible to laptop queries. Cheap now, and it keeps two unreconciled records. Rejected: it breaks G1 to G3 and the reconciliation in #9076 has nothing to reconcile against.

B. Port the native `fleet` binary to Workers unchanged. Rejected: most verbs need a worktree, a shell or a local socket (N1). The port would be large and would fail on those verbs.

C. Thin TypeScript CLI that calls a hosted HTTP service, which owns stamping and writes to Turso. Chosen (see section 4). The service is the only writer of stamps; the CLI is a client.

D. Thin TypeScript CLI that writes Turso directly. Rejected for stamps: any caller with the database credential could forge stamps (fails G4). Allowed only for read-only query paths under component G's prudence rules.

E. Mirror the laptop queue into Turso and make the mirror authoritative. Rejected for now: two queues break the single-queue rule in `doc/agents/one-queue-claiming-work.md`. See section 4.5.

## 4. Chosen approach

### 4.1 Split of duties

- Hosted service (Worker, route family `/fleet/v1/...`): the only stamp authority. It validates the caller, fills in stamp fields it knows, inserts the Metadata row in Turso in the same transaction as the provenance row, and then creates the GitHub issue or PR with the stamp written into the body metadata block. Stamp and insert succeed or fail together; a GitHub failure after insert leaves a row marked `pending_github` that a sweeper retries.
- Thin TS CLI (`fleet-hosted`): a client for the service. It holds no database credential and no GitHub token that can create issues on its own. It sends intent; the service stamps.
- Read path: Metadata query is a service endpoint that accepts the same filter expression as native `fleet` Metadata query. Direct Turso reads are allowed only for component G's read-only role.

### 4.2 Stamp parity

| Native field              | Hosted meaning                         | Treatment                                    |
| ------------------------- | -------------------------------------- | -------------------------------------------- |
| queue id                  | same                                   | kept                                         |
| scheduled start           | same                                   | kept                                         |
| pickup / work-start times | same, set by the service at claim time | kept                                         |
| worktree                  | no worktree on a Worker                | replaced by `origin` = `hosted:<service-id>` |
| host                      | Worker id                              | kept as `host`, value `workers:<region>`     |
| session id                | same                                   | kept                                         |
| model                     | same, from the caller's declaration    | kept, marked `declared`                      |
| lane-start issue set      | same                                   | kept                                         |

Unknown fields are rejected rather than silently dropped, so a caller cannot write a stamp field the service does not understand.

### 4.3 Auth and non-forgeable stamps

- Callers present a token from #9077 (relying-party token) or a service token for automation. The token names the caller and the scope it may stamp.
- The service sets every stamp field it can derive itself (timestamps, host, origin, caller identity). A caller can only supply the declared fields: queue id, session id, model, lane-start set.
- Stamps are signed by the service with a key held only by the Worker. The signature covers the stamp body and is written into the metadata block. Verification is a service endpoint so no caller needs the key.

### 4.4 Capability inventory against the native surface

| Native capability                  | Hosted equivalent                 | State                             |
| ---------------------------------- | --------------------------------- | --------------------------------- |
| Create issue/PR with stamp (#9073) | `POST /fleet/v1/issues`, `/pulls` | design here                       |
| Metadata query (#9074)             | `GET /fleet/v1/metadata?filter=`  | design here                       |
| Session provenance (#9075)         | `POST /fleet/v1/sessions`         | design here                       |
| Sync tools (#9016)                 | sweeper + `POST /fleet/v1/sync`   | design here, after the stamp path |
| Worktree, shell, local build       | none                              | stays native (N1)                 |
| Queue claim and complete           | see 4.5                           | open                              |

### 4.5 Queue answer

Recommendation: the hosted world does not run its own queue in the first phase. Hosted callers file intent through the service, and the service enqueues into the existing fleet queue through the same `repo-task/v1` packet schema. The laptop queue stays the single queue (per `doc/agents/one-queue-claiming-work.md`). A Turso-backed queue is deferred until a hosted worker needs to claim work with no laptop reachable; that case is an open question below.

## 5. Dependency-ordered build slices

Each slice is one lane or session. Each names its acceptance criterion and a done check.

1. S1, stamp schema and signing: define the stamp body, field table (4.2), and the signing scheme (4.3). Acceptance: schema file plus a unit test that rejects unknown fields. Done check: the test runs and passes.
2. S2, provenance and stamp table in Turso: migration for the metadata and provenance tables, with the `pending_github` state. Acceptance: migration applies on an empty database and rolls back. Depends on S1.
3. S3, create path: `POST /fleet/v1/issues` and `/pulls` with insert-then-GitHub and the sweeper retry. Acceptance: a test creates a stamped issue against a fake GitHub and asserts the row and body metadata match. Depends on S1, S2.
4. S4, query path: `GET /fleet/v1/metadata` accepting the native filter grammar. Acceptance: the same filter returns the same rows from native and hosted fixtures. Depends on S2.
5. S5, auth: verify #9077 tokens and service tokens; scope checks. Acceptance: a token without stamp scope gets 403 on create. Depends on S3.
6. S6, `fleet-hosted` CLI: thin client for S3 and S4. Acceptance: a CLI round trip creates a stamped issue in a test repo. Depends on S3, S4, S5.
7. S7, sync tools (#9016 counterpart): sweeper and sync endpoint. Acceptance: a `pending_github` row is completed by the sweeper after a simulated GitHub outage. Depends on S3.

## 6. Rollout and rollback

- Rollout is behind a flag per caller class. Phase 1: service token callers only, in a test repository. Phase 2: relying-party users, in a test repository. Phase 3: the production repository, one caller class at a time.
- Each phase starts with a read-only shadow: the service computes the stamp and writes it to a shadow table, but does not create the GitHub object. The shadow is compared with the native stamp for a fixed set of issues before the flag flips.
- Rollback: turn off the flag for the caller class. Created objects stay; rows in `pending_github` are retried or closed by the sweeper. The native `fleet` path is never removed by this work, so a hosted outage falls back to native filing.
- Schema rollback: S2's migration has a down step. Stamp signatures are kept so rows written during a rollback still verify.

## 7. Verification plan

- Unit: stamp schema rejects unknown fields; signature verifies and fails on any changed byte.
- Integration: create path against a fake GitHub, including the outage case that leaves `pending_github` and the sweeper that recovers it.
- Parity: a fixed filter set run against native and hosted fixtures returns identical rows.
- Security: a token without stamp scope cannot create; a caller cannot set service-derived fields; a forged signature fails verification.
- Live proof: after phase 1, one real issue created through the service is found by a native `fleet` Metadata query with its stamp intact. This is the acceptance proof for the design, to be run when S3 and S4 land.

Per `doc/agents/testing-doctrine.md`, the stamp schema and signature are property-tested where they have invariants, and the create path is model-based against the state diagram in S2 (`pending_github` to `done`). Each test names its oracle: the native stamp for parity, the signature check for forgery, and the fake GitHub for the create path.

## 8. Open questions

Each question names a decider. Per the no-owner-gates rule (FDR-0013), each has a default that the build proceeds on until the decider answers.

1. Where the hosted queue lives (section 4.5). Default: the laptop queue, via the service. Decider: the owner of #9078.
2. Whether relying-party tokens carry a stamp scope or whether stamping is a service-token-only action. Default: service-token-only until #9077 settles. Decider: owner of #9077.
3. Whether the stamp signing key is per service or per region. Default: one key per service deployment, rotated by the existing secrets process. Decider: owner of the hosted platform.
4. Whether `model` in the stamp is trusted when declared by a relying-party user. Default: marked `declared` and not used for any gate. Decider: owner of #9074 (Metadata query).
5. Whether the sweeper for `pending_github` runs on Workers cron or on the laptop. Default: Workers cron, with the laptop as a fallback runner. Decider: owner of #9016.

## 9. Record and tracking

- This document is the dated design record. It is linked from #9081.
- Build slices S1 to S7 are filed as granular implementation issues under #9081, per its acceptance list. Filing is tracked in the queue per `doc/agents/one-queue-claiming-work.md`.
- Tracking issue: #13873 (AUDIT-7 Lane 6).
