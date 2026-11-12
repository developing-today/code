# Decision Record: DoS Scenario Catalogue per Interface

Issue: [#5892](https://github.com/developing-today/code/issues/5892) — id: design: DoS scenario catalogue per interface (from #5685)
Task: [10502](https://github.com/developing-today/code/issues/5892) (queue repo-health-code-pilot)
Author: repo-worker-code, lane openchamber-scheduled-lane (8ac61616-3815-4e7b-b33d-840a3829154a)
Date: 2026-11-08

## Context and scope

Issue #5892 asks for a concrete, per-interface DoS scenario catalogue: scenarios, load profiles, pass/fail criteria, and recovery assertions. This is a design-only task; implementation is deferred to a future phase. The catalogue must not re-decide the parent programme decisions in issue #5685 (adversarial DoS suite and per-interface benchmark programme as CI regression gates sharing one load/measurement machinery).

Parent programme context (from #5685):
- One shared load/workload generator framework for both adversarial and benchmark programmes
- CI tiering: fast adversarial smoke tier on every change, heavier scheduled suite, and less frequent soak tier
- Regression gates: pass/fail expressed as server behaviour under defined load profiles; recovery within N seconds after load stops
- Blast-radius safety: dedicated fixtures, environment guards, load-generator isolation

Epic linkage: issue #2056 (OPEN).

## Decision

We adopt the following per-interface DoS scenario catalogue shape and criteria. Alternatives considered and rejected are noted below.

### Catalogue structure

For each transport/interface surface exposed by `pkgs/id`, the catalogue defines:

1. **Scenario name** – concise, descriptive
2. **Interface(s)** – transport(s) affected
3. **Threat class** – connection-level, request-level, protocol/interface, state-exhaustion, fairness, legitimate-surge
4. **Load profile** – concrete, unit-labelled parameters (arrival rate, payload size, duration, offered concurrency, and the relevant interface limit)
5. **Pass/fail assertion** – measurable server behaviour under load
6. **Recovery assertion** – server state and responsiveness after load stops
7. **Notes** – invariants, mitigations, or follow-up tickets

### Observed interface inventory

The source inventory below bounds the catalogue; optional/feature-gated surfaces are marked so a build that does not ship them is not falsely reported as covered. Line references are from the `main` worktree used for this decision.

| Surface | Concrete interface / wiring | Catalogue treatment |
|---|---|---|
| Iroh metadata | QUIC ALPN `/iroh-meta/3`; request/response wire enums in `pkgs/id/src/protocol.rs:270–371,413–498`; `id serve` registration in `pkgs/id/src/commands/serve.rs:1449–1453` | Include request parsing, policy-gated expensive/mutating operations, stream/session load and overload behavior. |
| Iroh blobs, docs, gossip | Built-in Iroh handlers registered by `id serve` in `pkgs/id/src/commands/serve.rs:1449–1453` | Inventory as production surfaces, but use upstream handler limits/fixtures; do not invent package-specific parser behavior. |
| Range event sync | QUIC ALPN `/id-range-sync/1`; `EventSyncProtocol` in `pkgs/id/src/event_sync.rs:95–169`; registered conditionally in `commands/serve.rs:1454–1461` | Include peer/session churn, bounded sync input, and synchronization under load; verify converged state after recovery. |
| Log sync API | QUIC ALPN `/id-sync/1`; `SyncProtocol` and public `sync_with`/`answer` in `pkgs/id/src/log_sync.rs:191–330` | Catalogue as a public library surface, but mark it **not registered by production `id serve`** at `commands/serve.rs:1449–1492`; only test it in consumers that expose it. |
| World sessions | QUIC ALPN `/id-world/1`; `WorldProtocol` in `pkgs/id/src/world_net.rs:119–166,267`; client/session APIs at `:705–769`; session framing/contract in `src/world_session/types.rs:39–64` | Include frame parsing, expensive RPCs, incomplete sessions, auth/capability boundaries and independent-caller fairness where those routes are enabled. |
| World replication | QUIC ALPN `/id-world-replication/1`; JSON request and handler limits/deadlines in `pkgs/id/src/world_replication_net.rs:41–120,315–342` | Include bounded request/frame/write work and post-load hash-chain/state verification. |
| Friend envelopes and artifact transfer | `/id-envelope/1`, `/id-artifact-push/1`, `/id-artifact-pull/1`; handlers/clients in `pkgs/id/src/envelope_net.rs:53–145` and `src/artifact.rs:67–96,982–1228` | Include duplicate/refusal paths, bounded upload/paging, stalled streams, and outbox pressure. |
| Presence over gossip | Signed heartbeat and table in `pkgs/id/src/chat_presence.rs:61–213`; strict frame decoder at `:456–487` | Include malformed/oversized heartbeat pressure, signature verification load, membership filtering and expiry/recovery. |
| Optional Axum HTTP/WebSocket | Routers in `pkgs/id/src/web/mod.rs:405–479`; route wiring in `src/web/routes.rs:487–608`; examples include multipart `/api/upload`, JSON file APIs, tags and identity APIs, and WebSockets in `src/web/tags_ws.rs`, `src/web/collab.rs`, and `src/web/world_ws.rs` | Separate HTTP body/parser, upload, request-amplification and WebSocket message/session rows; apply only when the `web` feature is built and enabled. |
| Optional SSH and SSH-over-Iroh | TCP SSH server in `pkgs/id/src/world_ssh.rs:166–195,326–518`; `/id-ssh-tunnel/1` at `:85–150`; world exec shares the world-session framing | Include auth/channel/session churn and bounded exec/tunnel traffic only in builds that enable these surfaces. |
| Hosted Cloudflare Worker | HTTP `fetch(Request, Env)` at `pkgs/id/hosted-worker/src/worker.ts:30–52`; validates `x-binding-id` / `x-jurisdiction` and forwards to a restricted Durable Object; routing validation at `hosted-worker/src/routing.ts:29–83` | Include header-validation and forwarding fan-out/timeout scenarios in hosted deployments; this is separate from the Rust Iroh server. |
| Outbound mail sink | `MailSink`/`SmtpSink` in `pkgs/id/src/directory_mail.rs:22–61,225–301`; SMTP is loopback-only and has a 10 s deadline / bounded reply parsing at `:205–223` | Not a remotely reachable ingress surface. Where an ingress operation can trigger mail, include bounded fan-out/deadline behavior in that operation's test; do not model the SMTP sink as a public listener. |

No gRPC or external message-broker handler was found in the package inventory. Internal Tokio channels are not network interfaces. The local CLI is also not part of this network-facing catalogue.

### Fixed initial fixture profiles and boundary values

These are bounded, reproducible initial gate profiles, not claims about production throughput. All use an isolated fixture, fixed seed, 30-second attack phase, and a 30-second recovery window. Deadline-bound scenarios are exempt from that fixed attack duration (see "Deadline-bound scenarios" below): a scenario whose pass condition requires observing a configured deadline expiry runs until that deadline elapses, or exercises the expiry through a supported test-only deadline override, instead of fitting inside the 30-second profile. The open-loop generator offers 128 operations/stream-open attempts per second; the canary offers 1 valid operation per second. Where an interface has an explicit cap, the fixture also tests exactly cap and cap+1; where it does not, the profile is a fixed 64 concurrent clients and the missing production bound is recorded as a contract gap. A profile is rejected at setup if its configured limit or unit is absent.

- **Iroh metadata `/iroh-meta/3`:** 64 concurrent peers, 128 request streams/s; test `MAX_REQUEST_BYTES - 1`, exact `MAX_REQUEST_BYTES` (1 MiB), and `MAX_REQUEST_BYTES + 1`; malformed postcard and truncated request at 1 in 10 offered inputs. The request cap is `pkgs/id/src/protocol.rs:264,839`; over-cap must be rejected before unbounded allocation, and valid metadata canary must still complete.
- **Built-in Iroh blobs/docs/gossip:** 64 concurrent peers, 128 stream/message attempts/s; exercise each upstream handler's configured request/message boundary and one-byte-over input, with 1 in 10 invalid inputs. Record the exact upstream limit in the fixture manifest; no Rust-package limit is presumed. Healthy blob/doc/gossip canary operation must remain usable.
- **Range event sync `/id-range-sync/1` and library log sync `/id-sync/1`:** 64 peers, 128 sync handshakes/s, with 1 in 10 malformed or refused handshakes; continue for 30 s. Valid canary peer must sync and converge within the recovery window. `/id-sync/1` remains library-only unless another consumer registers it.
- **World sessions `/id-world/1` and world WebSocket:** 64 sessions, 128 RPC/frame messages/s, 1 in 10 invalid; test framed message sizes 1 MiB−1, 1 MiB, and 1 MiB+1 (`MAX_FRAME_BYTES` in `pkgs/id/src/frames.rs:22`). Oversize frames are rejected/stream closed; valid canary RPC completes within its 5-second test deadline.
- **World replication `/id-world-replication/1`:** 64 peers and 128 request/frame attempts/s; invalid/oversize request cases are 1 in 10; stalled read/write is held across the implemented 30-second deadline (`pkgs/id/src/world_replication_net.rs:39,108,327–342`). Timed-out streams close; a valid hash-chain canary verifies after recovery.
- **Friend envelope and artifact push/pull:** 64 peer streams, 128 messages/s; frame sizes 1 MiB−1, 1 MiB, 1 MiB+1 and 1 in 10 malformed/duplicate/refused cases. Over-frame transfers are rejected; accepted push/pull/outbox canary completes without duplicate delivery or partial artifacts.
- **Presence over gossip:** 128 heartbeats/s for 30 s, including heartbeat with 64 channels and with 65; the channel bound is 64 (`pkgs/id/src/chat_presence.rs`). A correctly signed heartbeat with 65 channels is accepted with its retained channels truncated to 64 (`MAX_HEARTBEAT_CHANNELS`, `pkgs/id/src/chat_presence.rs` `update_at`); a valid member heartbeat remains visible and expires normally after load stops. Invalid-signature heartbeats are dropped separately, before that normalization path (`receive_presence_heartbeat`, `pkgs/id/src/commands/serve.rs:2190`).
- **Optional Axum HTTP and WebSocket:** 64 concurrent clients, 128 requests/messages/s; JSON body 10 MiB−1 / 10 MiB / 10 MiB+1 and upload image 10 MiB / 10 MiB+1; `/api/upload` is separately limited by its multipart allowance. These limits are `DEFAULT_BODY_LIMIT` / `MAX_IMAGE_SIZE` in `pkgs/id/src/web/routes.rs:43–57`; over-limit HTTP body returns 413. WebSocket frame cases use 1 MiB−1 / 1 MiB / 1 MiB+1 (`frames.rs:22`).
- **Optional SSH / SSH-over-Iroh:** test 64 concurrent TCP sessions and rejection at 65 (`world_ssh.rs:70–73`), 16 tunnel streams and rejection at 17 (`:90`), and packet sizes 32 KiB−1 / 32 KiB / 32 KiB+1 (`:70`). Over-cap opens/packets are refused/closed; a valid shell/exec canary completes after recovery.
- **Hosted Cloudflare Worker:** 64 concurrent requests, 128 forwarded requests/s for 30 s; invalid binding IDs and jurisdictions are 1 in 10; verify rejected input is not forwarded and valid Durable Object requests remain routable. This profile is separate from Iroh and uses the Worker route contract in `hosted-worker/src/routing.ts:29–83`.
- **Outbound mail path:** drive 128 mail-triggering ingress operations/s for 30 s in a loopback-only SMTP fixture; each sink operation remains under its implemented 10-second deadline and bounded reply-line/count limits. No live SMTP service is contacted.

### Deadline-bound scenarios (exemption from the fixed 30-second profile)

Some scenarios cannot, by construction, fit the fixed 30-second attack phase plus 30-second recovery window above: their pass condition is the expiry of a configured deadline `D_time` that is longer than 30 seconds. The concrete case in this catalogue is scenario 4 (slow/incomplete input) applied to the optional SSH interface: `pkgs/id/src/world_ssh.rs` configures a ten-minute inactivity timeout (`INACTIVITY_TIMEOUT`, wired as the session `inactivity_timeout`), so holding an incomplete input through the deadline cannot be observed in 30 seconds, and closing the attacking clients at 30 seconds would test client cleanup instead of server expiry.

Decision (issue #8064, resolving the Codex finding on PR #7738): deadline-bound scenarios are explicitly exempt from the fixed attack duration. For such a scenario the fixture must do one of the following, and record which in its manifest:

- **Extended profile (soak tier):** the attack phase runs until the configured deadline elapses (for SSH, ten minutes plus the recovery window), in the scheduled/soak tier rather than the fast gate; or
- **Deadline override:** the fixture configures a supported test-only clock/deadline override that shortens `D_time` for the test (for SSH, a shortened inactivity timeout), with the production deadline value recorded alongside, so the expiry assertion is exercised against the same code path at a test-scale duration.

A deadline-bound scenario is never satisfied by ending the fixture before the deadline, and its recovery window starts at deadline expiry, not at the 30-second mark.

### Concrete scenarios (per interface)

Each concrete row is bound to an interface adapter identified from the current code. A profile names the applicable limit and its unit (`C_rate`, `C_active`, `C_sessions`, `M_bytes`, or `D_time`) and the interface-specific overload response. These are explicit, versioned fixture inputs, not values silently inferred from each test run. This is a profile contract, not an assertion that all interfaces already expose identical production counters or limits: each adapter maps to the closest real limit/response in its handler; if a required bound or response is absent, that is recorded as a contract gap to resolve before implementation rather than guessed in a test.

Load generation uses a fixed **open-loop offered-load schedule**: it submits at the configured rate without slowing down in response to rejects, records offered/admitted/rejected/completed counts, and caps generator resources. `2×C` means twice the unit-labelled admission limit offered for the stated duration; it does not mean twice the number of successful completions. Every fixture records the literal numeric values and fixed seed.

| # | Scenario | Per-interface load profile | Pass/fail | Recovery |
|---|---|---|---|---|
| 1 | Connection/session churn (where the interface has connection/session state) | Offer new sessions at `2×C_rate` for 30 s while maintaining `C_sessions` valid sessions; use a fixed schedule to churn the remaining attempts | Attempts beyond the session/admission bound are refused or expired within `D_time`; existing valid sessions and a concurrent canary remain usable | After load stops, a fresh valid session succeeds; any observable transient session resources return to the pre-attack fixture baseline within 30 s |
| 2 | Input-boundary and malformed-message pressure | For 30 s, offer at `2×C_rate`; include sizes `M_bytes-1`, `M_bytes`, `M_bytes+1`, plus truncated/invalid encodings | Valid boundary sizes follow normal semantics; over-limit and invalid inputs are rejected at the interface boundary within `D_time`, with no panic or partial mutation | A subsequent valid message succeeds; observable transient parser/work resources return to the pre-attack fixture baseline within 30 s |
| 3 | Expensive-operation amplification | Select one real operation per interface from its handler/call path; offer valid requests at `2×C_rate` for 30 s, with concurrency capped at `C_active` and an independent canary at `C_rate/10` | Work is bounded by the declared limit; excess receives the documented overload result within `D_time`; accepted canary work continues; no unbounded queue/resource growth | Accepted work drains and the canary succeeds within 30 s; accepted state remains valid |
| 4 | Slow/incomplete input (where the interface supports streaming or incremental input) | Hold inputs incomplete through the interface's configured `D_time` idle/deadline boundary while maintaining valid canary traffic | Incomplete inputs are timed out/closed by the configured deadline; they cannot hold resources past their documented bound or starve the canary | Expired resources are released and a fresh valid input succeeds within 30 s |
| 5 | Stateful resource/quota exhaustion (where the interface exposes stateful writes or sessions) | Offer creations/writes at `2×C_rate` for 30 s against the fixture's explicit quota, with an independent valid canary below quota | Excess is refused atomically; no partial/corrupt records or unbounded queued work; unrelated fixture state remains readable | Accepted writes are readable, rejected writes left no partial state, and any observable transient resource returns to the pre-attack baseline within 30 s |
| 6 | Noisy-neighbour fairness (where the interface has independent principals/worlds) | Split valid traffic 90/10 between two isolated principals/worlds; offer the 90% share at `2×C_rate` for 30 s while the other sends at `C_rate/10` | The overloaded principal is bounded/rejected according to the interface contract; the canary principal continues to complete valid operations within its declared `D_time` | Both principals can make valid requests within 30 s after the flood; no cross-principal state or quota leakage |
| 7 | Legitimate-surge control | Ramp valid, representative operations from `C_rate/2` to `C_rate` over 30 s, then stop | In-cap valid traffic is not classified as malformed/hostile or rejected solely by a DoS guard; completion stays within `D_time` | Normal admission returns after the surge; no residual work remains beyond the documented drain bound |

Only applicable rows are instantiated for an interface; the catalogue records the reason a row is inapplicable rather than silently omitting it. Each row names the actual operation, route/message type, overload response, literal profile values and units for that interface. The implementation phase must not copy a numeric limit from another interface without evidence from this interface's contract or fixture.

| Interface family | Applicable scenario rows | Required specialization |
|---|---|---|
| Iroh metadata | 1, 2, 3, 7 | Metadata request variants and AccessPolicy-guarded mutations; use actual postcard request bounds and stream behavior. |
| Iroh blobs/docs/gossip | 1, 2, 5, 7 | Exercise upstream handler limits and package wiring; record upstream overload semantics rather than assuming Rust package handlers. |
| Range event sync and public log-sync API | 1, 2, 3, 7 | Peer stream negotiation, pull policy, event volume and convergence; note that `/id-sync/1` is not production `id serve` wiring. |
| World sessions and world replication | 1–7 where applicable | Session frame and RPC parsing, capability checks, JSON replication requests/frames, write deadlines, quotas, and recovery consistency. |
| Friend envelopes and artifact transfer | 1, 2, 3, 5, 7 | Envelope delivery/refusal, outbox pressure, signed push bounds, pull paging and stalled stream deadlines. |
| Presence over gossip | 2, 3, 7 | Signed heartbeat/frame decoding, channel-count bound, membership filtering and expiry. |
| Optional HTTP, tags/collaboration/world WebSockets | 1–7 where applicable | Multipart/JSON body parsing, file/blob operations, REST identity/tag endpoints, WebSocket message/session lifecycle; feature-gated. |
| Optional SSH and SSH-over-Iroh | 1–4, 6, 7 where applicable | Authentication, channel-open/exec/session framing, tunnel setup and configured packet/deadline bounds; feature-gated. |
| Outbound mail triggered by an ingress operation | 3, 5 | Bound fan-out and caller deadline; separately exercise the loopback SMTP sink's existing timeout/reply caps, not as a public listener. |

### Pass/fail criteria (common)

- **Safety and bounded work**: every scenario runs only against an isolated, disposable fixture with explicit time, connection, input-size, and work limits. The load generator refuses a shared/production target.
- **Correct overload behavior**: inputs beyond a declared limit receive the interface's documented rejection/close/backpressure response within its deadline `D`; the implementation does not accept work and then queue it without bound.
- **Honest canary**: a valid operation below the admission limit continues to complete within its declared per-interface deadline while the attack runs. A surface without an existing deadline needs an explicit deadline contract before its gate can be implemented.
- **No crash or corruption**: no panic, restart, unrecoverable error, partial mutation, or leaked cross-principal state.
- **Determinism**: fixed seed, fixture state, profile parameters, and machine-readable result for every run; no per-run auto-calibration of the gate threshold. Per-interface limit values are fixed in version-controlled fixture configuration.

### Recovery assertions (common)

- Within **30 seconds** after the attack ends, each affected interface accepts a fresh valid canary operation within its declared deadline `D`.
- All finite work queues drain. Where an adapter can observe transient connection/session, parser, or fixture-resource counters, it compares them with the idle pre-attack baseline; unobservable counters are not invented as an oracle and recovery is checked with interface probes and state assertions. Documented persistent state created by accepted operations is excluded from transient-resource comparisons.
- An independent read/consistency check confirms accepted state is intact and rejected writes left no partial state.
- No restart, manual cleanup, or fixture repair is needed. Failure to meet any recovery assertion is a test failure, not a reason to skip the result.

### Alternatives considered and rejected

| Alternative | Rejection rationale |
|---|---|
| One transport-neutral test with no per-interface adapter | Misses framing, lifecycle, limits, and overload semantics that differ between actual interfaces |
| Completely separate suite for each interface | Duplicates scenario intent and measurement semantics and lets results drift; use a shared scenario definition with thin adapters instead |
| Host-dependent raw throughput as the DoS pass/fail oracle | Sensitive to CI runner variance and does not prove bounded rejection or recovery; use explicit limits, deadlines, canaries, and recovery invariants |
| No recovery assertions | Recovery is part of the test; without it, a crash could be misread as a pass |
| Separate adversarial and benchmark suites with different machinery | Parent programme mandates shared load/measurement machinery to avoid drift |

### Invariants the implementation must hold

- **Shared machinery**: The adversarial suite and benchmark programme must use the same load/workload generator and results store
- **Per-interface gates**: Each interface’s regression gate uses the catalogue scenario tuned to that transport
- **No silent drift**: Baseline and thresholds are versioned; any change is a versioned event with announcement
- **Blast-radius safety**: Scenarios must never run against shared/production targets by accident; environment guards and dedicated fixtures are mandatory

### Splits and follow-up ownership

No additional split is needed for this catalogue. The shared generator, CI tiering, baseline, results store, and regression-gate policy remain in the parent programme's separate decomposition; this record does not create duplicate follow-ups or revise those decisions. Implementation of the adapters and scenarios is a later delivery step and is outside this design task.

### Links

- Parent programme: [#5685](https://github.com/developing-today/code/issues/5685)
- Programme epic: [#2056](https://github.com/developing-today/code/issues/2056)
- Task: [10502](https://github.com/developing-today/code/issues/5892)

---

**Decision recorded.** This closes the design work for issue #5892. No code changes are included in this task.
