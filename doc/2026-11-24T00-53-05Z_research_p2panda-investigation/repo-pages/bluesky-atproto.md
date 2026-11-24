# Bluesky / AT Protocol — a public-data protocol finishing itself, and what we borrow

Full lane notes: research lane BLUESKY for
[#13779](https://github.com/developing-today/code/issues/13779), from
public protocol sources only (specs, IETF charter, PLC announcements,
Bluesky engineering posts, ecosystem measurements). Read 2026-11-23.

**Shape of the answer.** AT Protocol was designed for *public* data —
public posts, public follows, public everything — and in 2026 it
essentially finished that design and handed it to formal governance. Its
delegated-authority answer is a hardened federated OAuth profile, which
is a different species from everything else in this bundle: authority
lives at a server, is granted interactively, and is revoked centrally.
For this estate the protocol itself is not a candidate substrate. Three
things are worth taking: the permission-set *vocabulary* idea, the
key-discipline patterns (cross-referenced from the survey), and one
concrete engineering task — **read compatibility for `id`**, filed as
[#13787](https://github.com/developing-today/code/issues/13787).

## State of the protocol, late 2026

- **The IETF ATP working group was chartered in March 2026.** Its scope
  is deliberately narrow: the repository data structure, firehose sync,
  AT URIs, and identifier-resolution requirements. Explicitly *out* of
  scope: OAuth, Lexicon, PLC, and non-public data. Read that scoping as
  the protocol's own statement of what is finished and universal versus
  what remains Bluesky-product territory.
- **PLC has been institutionalised.** The **PLC Organization is a
  registered Swiss association** (announced 2026-09-28) with a board
  mostly *not* drawn from Bluesky, preparing — not yet executing — the
  takeover of plc.directory. PLC read replicas shipped in February 2026,
  so the directory is no longer a single read point even before the
  legal handover completes.
- **Sync modernised.** Sync 1.1 arrived via **Tap** (December 2025) and
  the bsky.network relay upgrade (January 2026), easing the path for
  independent relays and AppViews.
- **The ecosystem is real and lopsided.** Community measurements
  (November 2026) count roughly **47.8M repositories**, of which about
  **96.9% still sit on Bluesky-operated PDSes**, across roughly **3,600
  independent PDS hosts**. Named independents with real user bases:
  **Blacksky** (~44.9K accounts, running its own relay and AppView and
  the Rust PDS "rsky"), **Eurosky**, and **Tangled**. The protocol's
  exit-and-rehost story is proven at small scale and untested at large
  scale — both halves of that sentence matter.

## The active frontier: "Spaces" (permissioned data)

Proposal **0016**, in alpha since **2026-08-20**, is ATProto's first move
beyond public data: **Spaces** — per-space gated repositories living
under a single space-authority DID. The crucial precision, from the
proposal itself: **access control here is not confidentiality.** Spaces
gate *who the PDS serves the data to*; there is **no end-to-end
encryption**, and credentials are app-bound and short-lived (~10
minutes). Spaces is therefore best understood as "private-ish groups on
trusted servers," architecturally closer to a Discord server than to
anything in the capability-auth half of this bundle. It validates the
demand for permissioned social data; it does not change what this estate
should build on — our permissioned-data story must be cryptographic
(the p2panda/Keyline/Meadowcap family), not server-gated.

## OAuth: Bluesky's actual delegated-authority answer

Since ATProto has no capability tokens, its delegation story is the
OAuth profile Bluesky hardened for federation — worth recording
precisely, because it is well-engineered *of its kind*:

- **OAuth 2.1 profile, strict**: authorization-code flow only, **PKCE
  mandatory**, Pushed Authorization Requests (PAR), **DPoP**-bound
  tokens (proof-of-possession, so stolen tokens don't replay).
- **`client_id` is a URL** pointing to a public client-metadata
  document — no central app registry; any client self-describes.
- Confidential clients authenticate with **private_key_jwt / JWKS**.
- **Permission sets** (shipped early 2026) are the genuinely novel part:
  protocol-native, named bundles of authority across five resource
  types — **repo, rpc, blob, identity, account** — referenced from
  client metadata via `include:<nsid>`, so an app asks for a *named,
  community-reviewable permission set* instead of composing raw scopes.

The structural contrast with the capability family is the finding:
authority in this model **lives at the PDS**, is granted in **one
interactive hop** by the account holder, **cannot be re-delegated or
attenuated by the holder of the grant**, is **revoked immediately and
centrally**, and has **zero offline or peer-to-peer story** — a client
with no network path to the PDS has no authority at all. That is a
rational design for "apps act on my social account," and a non-starter
for seats that must act while disconnected. **What we borrow is the
permission-set vocabulary idea**: named, versioned, reviewable bundles
of authority (`include:<nsid>` as a UX) are a better human interface to
a capability taxonomy than raw command lists — a presentation layer the
estate can put over UCAN commands or Keyline scopes without importing
the OAuth substrate.

## The filed task: `id` read compatibility (#13787)

The one piece of engineering this lane produced:
[#13787](https://github.com/developing-today/code/issues/13787) — *id:
AT Protocol read compatibility — DID resolution + repo/record ingestion
(consider)*, linked to epic
[#2056](https://github.com/developing-today/code/issues/2056) and to
#13779. Scope, as filed:

- **Handle → DID resolution with bidirectional verification** (the
  handle's DNS/well-known proof must match the DID document's claim),
  covering **did:plc** (whose strong form verifies the audit-log
  operation chain) and **did:web**.
- **Two read paths, honestly distinguished**: light XRPC reads (trust
  the serving PDS — fine for display) versus the **verifiable path**:
  fetch `com.atproto.sync.getRepo` as a CAR file, reconstruct the MST
  (Merkle Search Tree: SHA-256 key-depth fanout 4, prefix-compressed
  nodes), check root equality against the repo commit, verify the
  commit signature under the DID document's signing key, and enforce
  revision monotonicity. Only the second path gives `id` evidence-grade
  ingestion.
- **Optional relay firehose follow** for ongoing mirroring, behind the
  same verification.
- **Placement**: a new read-only importer crate, `id-import-atproto`,
  following the `id-import-notion` precedent — explicitly **not** in
  id-core and **not** in id-sync. Read compatibility is an ingestion
  adapter, not a change to the estate's identity model.
- **Rust candidates**: **atrium** (MIT) as the default — its
  `atrium-identity` and `atrium-repo` crates map to the two halves of
  the job; **jacquard** (MPL-2.0, zero-copy, performance-friendly,
  single-maintainer-shaped, pre-1.0) as the alternative if profiling
  demands it; **atproto-crates** (separable primitives, 0.14.x) for
  cherry-picked pieces.

## What this lane rules out

For the avoidance of doubt in future discussions: ATProto is not a
candidate for the estate's auth, sync, or storage layers — it is a
public-data social protocol whose permissioned-data frontier is
server-gated by design, whose authority model requires a live server,
and whose identity system (DIDs) is the only layer we interoperate with,
read-only, via #13787. Its governance maturation (IETF charter, Swiss
PLC association) is genuinely good news for the open social web; it
does not move any estate decision except confirming that DID-based
identity interop is a stable target worth the importer.
