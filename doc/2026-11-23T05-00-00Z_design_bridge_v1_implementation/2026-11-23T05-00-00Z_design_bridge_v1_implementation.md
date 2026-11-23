# The hosted signed-object bridge v1 — implementation record (#3818)

Implements #3818 from the #2046 design
(`doc/2026-11-01_design_id2046.md`) and the bridge-semantics
investigation (`doc/2026-10-29T11-27-00Z_investigation_bridge_semantics_hosted_native/`).
The #2276 proof of concept (`pkgs/id/tests/bridge_poc.rs`) is the
semantic oracle; this is its production shape in
`pkgs/id/hosted-worker`, beside the #3811 hosted data plane.

The dependency gate the issue names is discharged: #3808's binding
contract is closed (the registry record binds `binding_id` to the
account's current Iroh root key), and #3811's hosted per-user store
is accepted (the UserDO this bridge extends).

## What the bridge is

A sync peer, never a translator or a proxy (the investigation's
ruling). The hosted store holds signed objects and proof-carrying
chain/fork evidence; it assigns no order, re-signs nothing, and
serves no authoritative snapshot. A native node and the hosted
store converge by exchanging the objects themselves.

## Pieces

- `src/event.ts` — the signed event exactly as `chain.rs::decode`
  verifies it: canonical CBOR (the package's `canonical-cbor.ts`,
  plus chain.rs's 512 declared-length limit and NFC rule, checked
  over the tree), the p2panda header layout, the Ed25519 signature
  over the header-minus-signature encoding, BLAKE3 operation IDs
  (`src/blake3.ts`, pure TypeScript; Workers has no BLAKE3 and the
  package carries no runtime dependencies), and the ChainExt
  extension rules (domain, kind families and their one payload key
  each — including `chat.anchor.v1` → `anchor`).
- `src/bridge-store.ts` — the native ingest decision procedure
  (`decide_in`, the waiting pool, the release drain, fork proofs,
  the applied-prefix derivation) mirrored onto the Durable
  Object's SQLite, the mirror the PoC validated. The sync set is
  stored events + fork proof headers + waiting events (the PoC's
  set: a peer's waiting event is what unblocks this side, so the
  bridge serves it; `/id-range-sync/1`'s stored+proof set is the
  subset the native protocol needs because its peers hold their
  own waiting pools durably).
- `src/range.ts` — range reconciliation transcribed from
  `crates/id-events`' `range.rs`: keys, fingerprints, frames,
  bounds, budgets. K and T stay local; the wire is the same.
- `src/bridge-auth.ts` — session/call authentication: the routed
  binding guard every route already runs, plus an ID-key proof —
  an Ed25519 signature by the binding record's Iroh root key over
  a transcript naming the binding, a single-use five-minute
  nonce, the leg, and the body hash. No bound key on record →
  every bridge route fails closed.
- `src/bridge-session.ts` — the wire protocol (below), the server
  session the DO serves, and a reference client speaking the same
  frames (the native node's stand-in for tests; a production
  native node speaks these frames from Rust).
- `src/user-do.ts` — the routes: `GET /v1/bridge/challenge`,
  `GET /v1/bridge/heads`, `POST /v1/bridge/push`,
  `POST /v1/bridge/pull`, `GET /v1/bridge/ws` (schema v3 adds the
  bridge tables beside the existing ones). Bridge bytes count
  toward the per-user storage ceiling; a batch that would cross
  it is refused before anything is written. The front door
  forwards the bound key as the internal `x-hosted-iroh-id`
  header, stripped and re-set like the binding context.

## The wire

One ordered binary message channel (WebSocket in production),
one-byte tags: CHALLENGE/AUTH, HELLO (chain heads — the frontier
evidence), RESUME/RESUMED (a prior session's token plus positions;
positions are evidence, the range exchange re-derives truth),
RANGE flights, WANT/BATCH/WANT_END/ACK (bounded batches; every
batch acknowledged with its ingest outcomes, so a sender retries
exactly what was refused and a re-sent event is a duplicate),
DONE/TOKEN.

Because fork-blindness is real (the investigation, finding 2),
chain agreement runs the proof-carrying range exchange, never a
height comparison. Withholding is detected, not silent: at
session end, announced heads are read against the applied
frontier and the delivered set, producing the evidence record
(author, announced head, actual frontier, undelivered ids).

Bounds enforced on receipt: event bytes (64 KiB), batch events
(64) and bytes (1 MiB), want ids (512), frame cap (16 MiB), and
the range session budget (64 MiB by default, charged per entry).

## Boundaries, stated plainly

- **Payload semantics.** The codec enforces the header-level
  ChainExt rules and extracts what ingest's dependency rule needs
  (including a checkpoint's predecessor binding). The deep
  per-kind payload checks (tag op rules, document/note/link/
  rotation/chat content rules) are not re-implemented: an event
  failing only those is held here and refused natively. Only the
  key holder can produce such an event, and their own builder
  validates before signing — the divergence surface is a signer
  sabotaging their own chain, which fork/freeze rules then
  contain. A follow-up leg can port the payload checks.
- **Membership voiding.** The native layer that voids stored
  bodies needs bodies and group resolution; it is not part of
  the bridge store. Presence — what dependency and fork
  decisions turn on — is unaffected; projections resolve
  membership where they read.
- **The native dialer.** The hosted endpoint, the protocol core,
  and a protocol-faithful reference client land here; the Rust
  dialer inside id-sync (the `/id-range-sync/1` peer role over
  this wire) landed after, as `crates/id-sync/src/bridge.rs`: the
  same session the reference client speaks (CHALLENGE/AUTH,
  HELLO/RESUME, the range exchange over the native range core,
  bounded WANT/BATCH transfer, DONE/TOKEN, withholding detection),
  over a WebSocket transport, driving the native `EventStore`
  through a per-step store seam. Its tests run the dialer against
  a mirrored server session over an in-memory transport pair:
  convergence in both directions, resume, withholding evidence,
  and auth refusal. The cross-language fixtures
  (`pkgs/id/tests/bridge_fixtures.rs`) pin the contract underneath
  it: BLAKE3 agreement, TypeScript-signed events ingest natively,
  Rust-signed events verify here. The one-shot HTTPS legs
  (challenge/heads/push/pull) landed after the dialer, as
  `crates/id-sync/src/bridge_http.rs`: a `BridgeHttpClient` over a
  `BridgeHttp` request/response seam (reqwest in production),
  fetching a fresh single-use nonce per leg and signing the same
  bridge-auth transcript; its tests run the legs against a mirror
  of the hosted routes over a native store — round trip, nonce
  replay refusal, pagination, refused outcomes. Still open: a live
  native-against-hosted session test.
- **WebSocket hibernation.** Sessions are served on the classic
  accept() path; a DO eviction mid-session drops the session and
  the client's retry resumes it. Hibernation-API session
  survival is a follow-up.
- **Sequence numbers** above the safe-integer limit are refused
  rather than stored imprecisely (SQLite integers); no real
  author approaches it.

## Evidence

- Hosted-worker suite: the bridge suites (BLAKE3 vectors, event
  codec incl. bad-signature / wrong-key / malformed negatives,
  range simulation to exact symmetric difference, store semantics
  incl. fork/freeze/waiting caps/expiry/checkpoint predecessors,
  session acceptance — convergence both directions, equal-height
  fork by proof exchange, withholding detection, replay identity,
  retry/resume, budget refusal — and the DO routes incl. proof
  negatives and the ceiling) run with the package's `npm test`.
- Rust fixtures: `cargo test -p id --test bridge_fixtures`
  (manifest `pkgs/id/Cargo.toml`) — BLAKE3 cross-pins and
  TypeScript-signed events decoding/ingesting natively.
