# Design: fleet second version in TypeScript — scope, WASM evaluation, transports, direct-Turso prudence

- Date: 2026-11-24 (UTC)
- Source issue: developing-today/code#9085 (component G of epic #9078, hosted editions program)
- Related: #9078 (program), components A #9079, B #9080, C #9081, D #9082, E #9083, F #9084, H #9086, I #9087; #5671 (ID transport fallback for UDP-denied networks); intake note `doc/2026-11-12T23-04-00Z_design_hosted-editions-workers/note.md` (statements S11–S14, S18–S19)
- Status: DESIGN (no implementation). Evidence limits are stated where they apply; every "to measure" item is an open question with a named decider.

## 1. Problem and context

The laptop PM stack, `pkgs/id` and the fleet tool run as native Rust binaries and the laptop queue. None of them run on Cloudflare Workers (the hosted-editions target, per the intake note). Workers limits (CPU time per request, no `std::fs`, limited native crypto and threading, small bundle budget) mean the hosted editions cannot reuse the Rust binaries as-is. The consequence, stated plainly by the owner (2026-11-12), is that the hosted editions are substantially a second implementation in TypeScript.

What breaks today if this is not designed:

- Two implementations of the same semantics (Metadata, repo-task/v1 packets, store contracts) drift silently. Nothing currently pins them to one another.
- Without an explicit transport contract, each component invents its own HTTP/WebSocket surface, and offline/reconnect behaviour differs by component.
- Without a rule for direct database access, clients may reach Turso directly for operations that need stamping or cross-tenant checks, bypassing the service layer.

## 2. Goals and non-goals

Goals:

1. Decide, per Rust component, whether logic is compiled to WASM and reused, rewritten in TypeScript, or kept server-side only.
2. Define the base-function inventory per component and the transport contract for each (request/response, streaming, realtime sync), including offline and reconnect behaviour.
3. Define "prudent" for direct Turso access: which operations may go direct, under which credential scope and expiry, and which must never bypass the service layer.
4. Define how the TypeScript and Rust implementations keep one data model: shared schemas, fixtures and conformance tests, and what happens when they diverge.

Non-goals:

- Implementing any component. Slices in §5 are the implementation plan, not work done here.
- Redesigning identity (owned by #9077 / #9086) or credential issuance (owned by E #9083). This doc consumes their outputs.
- Choosing the storage backend for media (F #9084) or the self-hosted tier (I #9087).
- Porting the Rust binaries "down". Both editions are tier 1 (intake S18); the TypeScript edition is a parity implementation with deliberate divergence where Workers cannot support a feature.

## 3. Alternatives considered

| Option                                                                                                                                                                              | What it means                                               | Tradeoff                                                                                                                                                                |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A. Do nothing; each component ships its own TypeScript port ad hoc                                                                                                                  | Fastest for one component                                   | Drift is near-certain; transport and direct-access rules become per-component decisions; rejected.                                                                      |
| B. Compile all Rust core to WASM and run it on Workers and in browsers                                                                                                              | Single source of semantics                                  | Crypto correctness risk is concentrated in one blob; bundle and cold-start cost; WASM cannot do the I/O the Workers runtime does; not viable for server-side I/O paths. |
| C. Rewrite everything in TypeScript; no WASM                                                                                                                                        | One runtime, simple deployment                              | Duplicates crypto and canonical encoding by hand; highest correctness risk for ID crypto; no reuse of tested Rust code.                                                 |
| D. Per-component decision (chosen): WASM only where the logic is pure, deterministic and bounded; TypeScript rewrite for I/O and orchestration; server-side only where secrets live | Reuse where cheap and safe, rewrite where Workers forces it | Requires a conformance suite to prevent drift (§4.4); evaluation work per component.                                                                                    |
| E. Do-nothing-plus-pinning: keep Rust as sole source, generate TypeScript types only                                                                                                | Keeps semantics in one place                                | Does not cover behaviour, only shapes; insufficient for Metadata and packet semantics.                                                                                  |

Chosen: option D, with the conformance suite from §4.4 as a precondition for every TypeScript rewrite.

## 4. Chosen approach

### 4.1 Rewrite-vs-WASM per component

Criteria, applied in order: (1) crypto correctness risk, (2) maintenance divergence between two implementations, (3) bundle and runtime limits on Workers and browsers, (4) whether the logic does I/O at all.

Rule: logic that is pure, deterministic and has no secrets in scope may be compiled to WASM and reused. Logic that does I/O, orchestration or holds secrets is rewritten in TypeScript or stays server-side.

| Component                     | Rust logic                                                  | Verdict (proposed)                                                                                                                                    | Evidence needed before commit                                                                                              |
| ----------------------------- | ----------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| ID crypto (`pkgs/id`)         | key derivation, signing, canonical encoding                 | WASM reuse for pure primitives (hash, canonical encoding, verify); TypeScript only for thin wrappers; signing with server-held keys stays server-side | Measured WASM size and cold-start on Workers; audit of the chosen crypto crate's WASM build; test vectors shared with Rust |
| ID store logic                | redb/Turso store contract, sync cursors                     | TypeScript rewrite behind the storage interface (intake S5); not WASM, because it is I/O-bound                                                        | Store conformance suite passing on both backends                                                                           |
| PM aggregator / reconcile (A) | board registry, hierarchy, Metadata                         | TypeScript rewrite; Metadata encoding shared via WASM only if a pure encoder is extracted                                                             | Metadata fixture parity (§4.4)                                                                                             |
| Fleet tool (C)                | Metadata stamping and query, session provenance, sync tools | TypeScript CLI and service; stamping logic shares the pure encoder if extracted                                                                       | Stamping parity fixtures                                                                                                   |
| Packet schema (repo-task/v1)  | JSON schema and validation                                  | Shared JSON Schema and fixtures; validators generated or hand-written per language, checked against the same fixtures                                 | Schema compiles to both validators                                                                                         |
| Browser client                | any local crypto or parsing                                 | WASM allowed for pure primitives only, behind feature checks                                                                                          | Bundle budget per the owner's limit (to be set by E)                                                                       |

Open question Q1 (decider: owner of #9085 with the crypto reviewer): whether any ID crypto primitive is so security-sensitive that it must stay native-only even on the client. Default until decided: WASM is not used for signing; only verification and canonical encoding may use WASM.

### 4.2 Base-function inventory and transport contract

Transports: every base function must be reachable over REST (HTTP/1.1 and HTTP/2 minimum) or WebSocket. HTTP/3 and QUIC are used where the platform provides them and are never required by the contract (Workers and browsers vary). The contract is "one of REST or WebSocket must work", with the transport chosen by capability negotiation, not by component.

Interaction classes:

- Request/response: reads, single writes, validation, stamping. Served over REST; idempotency keys required on writes.
- Streaming: long-running reads, log tails, queue status. Served over server-sent events on REST or as a WebSocket stream.
- Realtime sync: multi-writer state (board changes, queue claims). Served over WebSocket with a sequence cursor; the server is the ordering authority.

Base-function inventory (per component; the full table is the first slice's output):

| Component | Base functions (minimum)                                              | Class                                                                        |
| --------- | --------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| A PM      | list boards, read item, write item, reconcile run, dashboard read     | request/response; reconcile is streaming; dashboard subscription is realtime |
| B ID      | sign, verify, store put/get, sync pull, sync push                     | request/response; sync is realtime                                           |
| C fleet   | stamp, query Metadata, session provenance write, queue claim/complete | request/response; queue status is streaming                                  |
| E tenancy | route resolve, credential issue, credential refresh                   | request/response                                                             |
| F content | put object reference, get reference, publish                          | request/response                                                             |

Offline and reconnect: clients keep a local operation log with idempotency keys. On reconnect they replay the log from their last acknowledged sequence. The server returns the authoritative sequence; conflicts are resolved by the owning component's rule (for Metadata, last-writer-wins by server sequence; for queue claims, the atomic claim wins). No component may invent a different reconnect rule without an entry in this table.

Transport fallback for networks that deny UDP (#5671): WebSocket over TCP is the mandatory fallback; QUIC is an optimisation only.

### 4.3 Direct Turso access: what "prudent" means

Direct access means a client or non-service process opening a Turso connection rather than calling a service.

May go direct (when E's credential is scoped to the user's own database or the public database):

- Reads of the caller's own rows and of public-database rows.
- Sync pulls of rows the caller already owns, where the row carries no server-stamped field the client is trusted to produce.
- Writes that are pure self-owned content with no stamping, no authorisation beyond row ownership, and no cross-tenant effect.

Must never bypass the service layer:

- Any write that requires server stamping (Metadata stamps, sequence numbers, provenance).
- Any authorisation beyond the database scope (roles, group membership, delegation).
- Any cross-tenant access, including reading another user's database.
- Queue claims and any operation whose correctness depends on atomic multi-row ordering owned by the service.
- Any operation using credentials that are not scoped and expiring per E's model.

Credential rule: direct tokens are short-lived, scoped to one database, and re-issued through the service. A leaked direct token must be bounded by its expiry and scope; no direct credential may be a master credential.

Open question Q2 (decider: E #9083 owner): exact expiry window and the revocation path for direct tokens. Default until decided: 1 hour expiry, revocation by rotating the per-database token through the tenancy service.

### 4.4 Shared-semantics discipline

- Shared schemas and fixtures live in one directory and are the single source: Metadata, packet schema, store contracts, transport messages.
- Each language has a conformance suite that runs the same fixtures. The Rust suite and the TypeScript suite both must pass before a change to a shared fixture merges.
- Divergence policy: a divergence is a bug unless it is listed as deliberate in a register with a reason (Workers cannot support the feature). Deliberate divergences never remove a base function from either edition (intake S18).
- A fixture change that breaks one implementation blocks the PR until both are updated in the same change.

Open question Q3 (decider: owner of #9085): where the shared fixture directory lives and whether it is a separate package. Default: `doc/` is not used; the directory lives beside the shared schema package in the repo.

## 5. Dependency-ordered build slices

Each slice is sized for one lane or session and has its own acceptance criteria. Slices 1 and 2 are design-only.

1. **Evidence pass.** Measure WASM size and cold-start for the chosen ID crypto primitives on Workers; list the Rust logic inventory per component with file:line. Acceptance: a table in this doc or a child doc with measured numbers and no unmeasured claims.
2. **Base-function inventory.** Turn §4.2 into a per-component table with request/response, streaming and realtime classes, idempotency rules and reconnect rules. Acceptance: every base function in A–F has a row and a class.
3. **Shared fixtures and schema.** Create the shared fixture directory with Metadata and repo-task/v1 fixtures; wire both language suites to it. Acceptance: both suites run the fixtures in CI and pass on main.
4. **Prudence register.** Encode §4.3 as a checked list in the tenancy design (E) and a test that a direct-access path cannot perform a stamping write. Acceptance: test fails when a stamping operation is added to the direct list.
5. **WASM spike for ID verification and canonical encoding.** Build the WASM module, run the shared vectors, measure. Acceptance: vectors pass in WASM and in Rust; size and cold-start numbers recorded.
6. **TypeScript rewrite of PM and fleet services** behind the shared fixtures (slices owned by A and C). Acceptance: fixture parity, no divergence without a register entry.
7. **Transport layer.** REST and WebSocket adapters for the base functions with sequence cursors and the TCP fallback from #5671. Acceptance: reconnect replay test with a dropped socket, and a UDP-denied network test.

## 6. Rollout and rollback

Rollout: slices 1–4 are design and tests only and ship first. Slice 5 (WASM) ships behind a feature flag for verification only, not for signing. Slices 6–7 roll out per component (A, then C, then B), each behind a per-tenant flag, starting with the owner's own tenancy.

Rollback: every component keeps the Rust path until its TypeScript edition passes fixture parity for two consecutive releases. Rolling back a component means routing its tenants back to the Rust path; no data migration is required because the store contract is shared.

Abort criteria: a fixture divergence in a stamping path, or a direct-access path that reaches a stamping write, stops the rollout of that component.

## 7. Verification plan

- Fixture parity: both suites run the shared fixtures on every change to a shared schema or fixture.
- Property tests (per the testing doctrine) for canonical encoding round-trips and for sequence replay: any operation log replayed after reconnect yields the same state as an uninterrupted run.
- Model-based test for the reconnect protocol against the §4.2 rules.
- Negative test for §4.3: a direct-access token attempting a stamping write is rejected.
- Live proof: one hosted tenant run end-to-end over WebSocket with a dropped connection, and one over REST on a UDP-denied network, recorded with timestamps and the sequence cursor before and after.

Every automated test names its oracle: fixture parity uses the Rust result as oracle; replay uses the uninterrupted run; the direct-access negative test uses the expected rejection code.

## 8. Open questions

| #   | Question                                                                | Decider                                        | Default until decided                                                                     |
| --- | ----------------------------------------------------------------------- | ---------------------------------------------- | ----------------------------------------------------------------------------------------- |
| Q1  | Must any ID crypto primitive stay native-only, including on the client? | Owner of #9085 with the crypto reviewer        | WASM is used for verification and canonical encoding only; no WASM signing.               |
| Q2  | Expiry window and revocation for direct Turso tokens                    | Owner of E #9083                               | 1 hour expiry; revocation by rotating the per-database token through the tenancy service. |
| Q3  | Location and packaging of the shared fixture directory                  | Owner of #9085                                 | Lives beside the shared schema package, not under `doc/`.                                 |
| Q4  | Browser WASM bundle budget                                              | Owner of E #9083 with the browser client owner | Pure primitives only; no budget set, so measure in slice 5 and record.                    |
| Q5  | Whether HTTP/3 is in the required transport set                         | Owner of #9085                                 | Not required; REST and WebSocket are the contract, QUIC is an optimisation.               |

## 9. Source and status

Source issue #9085 is linked to this doc. The source issue's acceptance items are: this design merged under `doc/`, and granular implementation issues filed under #9085 or handed to the owning component issues with cross-links. The second item is handled by the slices in §5, to be filed when slice 1 is accepted.
