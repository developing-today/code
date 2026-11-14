# Research: identity, capabilities, petnames, transport

Date: 2026-10-10. Scope: OCapN, keyhive, p2panda, Spritely Goblins, petnames. Question per item: is there a Rust crate we could import, under what license, and how mature is it? Then recommendations for three uses in `pkgs/id`:

- (a) capability-style friend and document authorization
- (b) local petnames for accounts, with disambiguation
- (c) transport over iroh

Method: crates.io JSON API and raw or repo pages via webfetch. No code changed, no cargo run, no commits.

## Summary

- Capabilities: borrow keyhive's convergent-capability model (signed, content-addressed grants and revocations). Do not import `keyhive_core` yet. Pre-alpha, not audited.
- Petnames: borrow the concept from Stiegler's 2005 paper and implement a small local map. No suitable Rust crate exists. Do not use the `petname` crate; it is a random name generator.
- Transport: keep iroh 1.x, which `pkgs/id` already depends on. Borrow sync ideas from p2panda or subduction rather than importing a stack that is pre-stable.

## Per item

### 1. OCapN (Object-Capability Network)

- What: a spec for capability-secure messaging across networks (CapTP-style references, promise pipelining, netlayers). Unifies Agoric, Spritely, and Cap'N Proto CapTP ideas.
- Spec status: `github.com/ocapn/ocapn`, Apache-2.0, 217 commits, pre-specification. The repo says AI-authored spec text is not accepted.
- Rust crates:
  - `ocapn-netlayer` 0.1.4, MPL-2.0, last published 2025-01-30, about 3.7k downloads. Repo `codeberg.org/cmars/ocapn-rs`, 13 commits, README says "Stage 0 in progress".
  - `ocapn-syrup` 0.2.0 (Syrup serialization), `ocapn-agent` 0.1.0 (placeholder).
  - No crate named `ocapn` exists on crates.io.
- Verdict: do not import. Too early, and MPL-2.0 is copyleft at file level. Keep the ideas (sturdy refs, pipelining) as reference only.

### 2. keyhive (Ink & Switch)

- What: capability-based access control for end-to-end encrypted, local-first data.
  - Convergent capabilities ("concap"): signed, content-addressed delegations and revocations that form a CRDT. Reachability decides whether a grant is valid. Attenuation only, never escalation. Access levels Relay < Read < Edit < Admin.
  - BeeKEM: concurrent TreeKEM for group keys, with causal encryption.
  - Ciphersuite: XChaCha20-Poly1305 with a synthetic-nonce construction. Not independently reviewed.
- Crate: `keyhive_core` 0.6.0, Apache-2.0, published 2026-09-25, `rust_version` 1.90.0, about 15k lines.
- Maturity: repo says "DO NOT use this release in production". Not audited. Pre-alpha.
- Sync layer: `subduction_core` 0.19.0 (MIT OR Apache-2.0, 2026-10-07), "sync protocol for replicating Sedimentrees between peers". Not fetched or inspected in this pass.
- Verdict:
  - Do not import `keyhive_core` now. The API and wire format will move, and the crypto is unaudited.
  - Borrow the concept of signed, attenuable, revocable grants (see recommendation a).

### 3. p2panda

- What: operation-based (DAG of signed ops) group membership and access control. Also sync, blobs, and encryption layers.
- Crates, all 0.7.1:
  - `p2panda-auth`: MIT OR Apache-2.0, edition 2024, published 2026-08-21. Access levels Pull, Read, Write, Manage. Has a "strong removal" resolver. Cites keyhive as a different capability-chain model.
  - `p2panda-core`, `p2panda-sync`: MIT OR Apache-2.0.
  - `p2panda-net`: MIT OR Apache-2.0. Default features are `iroh_endpoint`, `iroh_mdns`, `gossip`, `discovery`, `sync`, so it pulls in iroh.
- Maturity: README says "APIs not yet stable". Versions are 0.x.
- Verdict:
  - `p2panda-auth` is importable and permissively licensed, but its model is membership-as-ops, not capability chains. Pre-stable APIs are a churn risk.
  - `p2panda-net` is the nearest importable iroh wrapper. Same churn risk, and it brings its own opinions about discovery and sync.

### 4. Spritely Goblins

- What: an object-capability runtime with actors, vats, and OCapN work. Relevant as a reference for object references and persistence.
- Location: `codeberg.org/spritely/goblins` (the GitHub mirror README returned 404).
- Language: Guile Scheme, not Racket. The task description said Racket; the repo says Guile.
- License: Apache-2.0. About 1,969 commits. Contains an OCapN test suite under `tests/`. Noise netlayer work in October 2026. Self-described as "very early".
- Crate: none. Not importable into Rust.
- Verdict: reference only. Borrow the object-reference and petname-system ideas. Do not depend on it.

### 5. Petnames

- Concept source: Marc Stiegler, "Introduction to Petname Systems" (2005, updated 2010, MIT X license). Read in full this pass. Key points:
  - A petname is a private, bidirectional mapping from a key to a name. Everywhere the UI shows the key, it shows the petname.
  - Nicknames and alleged names are public, not unique, and must be visually distinct from petnames.
  - Petnames are guessable, so the mimicry risk (for example "PayPa1" vs "PayPal") has to be checked when a name is created.
- Requirements the paper lists for a real petname system:
  1. One petname per key per user per app, and one key per petname.
  2. Names are assigned only by explicit user action, and can be edited repeatedly.
  3. The UI helps prevent names that are similar enough to be mimicked.
  4. Nicknames and alleged names are visually distinct from petnames.
- Spritely's 2022 post on petname systems (Jessica Tallon) adds edgenames (shared between two parties) and salted monster avatars for visual disambiguation. Read in an earlier pass.
- Crates:
  - `petname` 3.2.0 (allenap/rust-petname, Apache-2.0): a random name generator. Not the petname concept. Do not use.
  - `onomancy_automerge` 0.3.0 (inkandswitch/onomancy, MIT OR Apache-2.0, 2026-09-03, edition 2024). Described as an Automerge substrate adapter with petnames. About 56 downloads. Depends on Automerge, so it is a heavy dependency for a small feature. Not inspected.
  - `gaz` 0.0.1 and `insigne` 0.0.1: name-reservation crates, merely made. Not inspected.
- Verdict: no suitable crate. Implement the concept locally. Read `onomancy` only if we later want a shared-edgename design.

## Recommendations

### (a) Capability-style friend and document authorization

- Borrow keyhive's concap model. Each grant is a signed record:
  - issuer key
  - subject key
  - resource id
  - access level (Relay, Read, Edit, Admin)
  - parent grant hash, so the chain is content-addressed
  - expiry or none
- Rules:
  - A grant may only attenuate: a subject can receive at most the level its issuer holds.
  - Revocation is a signed op that references a grant hash.
  - A grant is valid when its chain reaches the resource owner and no link is revoked.
- Use the Ed25519 signatures and postcard encoding that `pkgs/id` already uses (`ed25519-dalek` 3, `postcard`). No new crypto dependency is needed.
- Do not import `keyhive_core` or `p2panda-auth` now. Revisit when keyhive reaches a stable, audited release, or when we need group key agreement, which is the part keyhive is actually solving.
- Scope: this is a small design. Write it as a separate doc before code.

### (b) Local petnames for accounts, with disambiguation

- Store a local map of public key to petname, enforcing both directions of uniqueness from Stiegler's requirements.
- Never show a remote's self-chosen name in the petname position. Show it only as an alleged name, in a visually distinct style.
- On creating or editing a petname, run a cheap similarity check against existing petnames. Stiegler's paper says a weak check is acceptable: over-warn rather than miss a mimic. Example: case-folded edit distance plus confusable-character normalization (`1`/`l`/`I`, `0`/`O`).
- For disambiguation when two contacts share a petname, the cheapest option is a short key fingerprint suffix. A salted visual avatar (Spritely's monster-avatar idea) is optional later.
- No crate needed. Roughly a small table plus one function.

### (c) Transport over iroh

- `pkgs/id` already depends on `iroh = "1"`, `iroh-gossip`, `iroh-blobs`, and `iroh-docs`. Keep iroh as the transport. Its QUIC endpoint and relay story are the right base.
- For peer discovery, `iroh-mdns-address-lookup` is already present. Use it for LAN.
- For sync, borrow the design from p2panda-sync or subduction, not their full stacks. `p2panda-net` would pull in pre-stable APIs and its own discovery and sync choices.
- Check version compatibility between `iroh` 1.x and `iroh-gossip` 0.101 before any upgrade. Not verified in this pass.

## Discrepancies

- Goblins is Guile, not Racket (task description said Racket). Repo: `codeberg.org/spritely/goblins`.
- `ocapn.io` was unreachable (transport error). The spec was read from the `github.com/ocapn/ocapn` README instead.
- The GitHub raw README for Goblins returned 404. Used the Codeberg page.
- No crate named `ocapn` exists. The OCapN crates are `ocapn-netlayer`, `ocapn-syrup`, and `ocapn-agent`.
- The `petname` crate is a name generator, not the petname concept.
- websearch returned HTTP 503. Used webfetch against crates.io APIs and repo pages instead.
- The `iroh` 1.3.0 "about 3.1M downloads" figure came from a truncated saved output and was not reconfirmed.

## Unverified

These claims were not confirmed by reading the source or a current fetch:

- `iroh` 1.3.0 download count (about 3.1M), and its exact latest version.
- `subduction_core` internals. Only the crate description was read.
- keyhive's ciphersuite and nonce construction. The repo says it is not independently reviewed, but the construction was not checked.
- `keyhive` "Keyline" design doc: not read.
- OCapN spec text: not read directly, because `ocapn.io` was unreachable.
- `onomancy_automerge` behavior, API, and whether it works without Automerge: not inspected.
- `gaz` and `insigne` name-reservation crates: not inspected.
- Spritely's 2022 petname post: details come from an earlier read, not re-fetched this pass.
- `iroh-gossip` 0.101 compatibility with `iroh` 1.x: not checked.
- Goblins' Noise netlayer status (October 2026): from the Codeberg page, not verified against commits.
- Existing docs in `/home/user/work/docs-artifacts/doc/` (`2026-10-09T09-59-07Z_design_attenuated_capabilities`, `2026-10-10T04-58-36Z_design_signed_artifacts`) were not read. They may overlap with recommendation (a).

## Sources

- crates.io API: `keyhive_core`, `p2panda-auth`, `p2panda-core`, `p2panda-sync`, `p2panda-net`, `subduction_core`, `iroh`, `ocapn-netlayer`, `petname`, `onomancy_automerge`
- `github.com/inkandswitch/keyhive` design docs (`threat_model.md`, `ciphersuite.md`, `README.md`, `convergent_capabilities.md`)
- `github.com/p2panda/p2panda` README; `p2panda-auth` README
- `github.com/ocapn/ocapn` README
- `codeberg.org/cmars/ocapn-rs`; `codeberg.org/spritely/goblins`
- Marc Stiegler, "Introduction to Petname Systems" (http://www.skyhunter.com/marcs/petnames/IntroPetNames.html)
- `spritelyproject.org` petname-systems post (2022)
- `pkgs/id/Cargo.toml` (existing dependencies)
