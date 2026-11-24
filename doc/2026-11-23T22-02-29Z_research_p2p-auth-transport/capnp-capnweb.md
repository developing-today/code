# Cap'n Proto / Cap'n Web — the live object-capability programming model

Full lane notes: Part 1 of research lane CAPNP-SURVEY for
[#13779](https://github.com/developing-today/code/issues/13779), from
project docs, specs, repos, and crates.io, read 2026-11-23. The wider
token-field survey from the same lane lives in
[Biscuit and the token family](biscuit-and-token-family.md).

## What the two pieces are

**Cap'n Proto** is two things sharing a schema language: (a) a binary
**serialization format** whose wire layout is the in-memory layout —
fixed widths, fixed offsets, offset-based pointers, little-endian — so a
message needs _no_ encode/decode pass and can be mmap'd, randomly
accessed, and read incrementally; and (b) an **RPC system** in which
interface references are a first-class type in the schema, so a method
parameter or return value can be a live reference to a remote object
([capnproto.org](https://capnproto.org/),
[RPC protocol doc](https://capnproto.org/rpc.html)). It was written by
Kenton Varda (primary author of Protocol Buffers v2) and is now
maintained by the Cloudflare Workers team; it is the RPC substrate inside
`workerd` and was built for Sandstorm.io.

**Cap'n Web** is a separate, younger protocol by the same author: a
schema-free, JSON-based object-capability RPC implementation in pure
TypeScript for browsers and web servers, announced by Cloudflare in
September 2025 and explicitly "experimental"
([Cloudflare blog: Cap'n Web](https://blog.cloudflare.com/capnweb-javascript-rpc-library/)).
It is _not_ wire-compatible with Cap'n Proto; it is a sibling design that
borrows the capability model and promise pipelining and drops the schema
and the binary format. It interoperates with Workers RPC (which is itself
Cap'n Proto under the hood) through workerd.

## The serialization layer (brief, because it is not the decision)

- Zero-copy: there is no encoding/decoding step; the Cap'n Proto
  encoding is appropriate both as a data interchange format and an
  in-memory representation. A "packing" codec strips zero bytes to reach
  protobuf-class wire sizes while staying faster than a parse-based
  format.
- Schema evolution is structural: fields are numbered in the order added,
  new fields go at the end, readers bounds-check — old and new schemas
  interoperate without a version negotiation. Struct-list
  canonicalization exists specifically to make Cap'n Proto more
  appropriate for use in cryptographic protocols — i.e. a canonical byte
  form you can sign.
- Accessors validate pointers before following them; the project has had
  fuzzing and expert security review.

Estate relevance: Cap'n Proto serialization is a fine fit anywhere the
estate wants a fast, signable, schema-evolving binary message format, and
it is _already_ the format under Workers RPC. But serialization is not
what this investigation is deciding; the RPC capability model is.

## The RPC capability model

Cap'n Proto RPC is a distributed-object protocol descended from CapTP
(the E language's protocol). The core rule, from the spec doc
([capnproto.org/rpc.html, Security](https://capnproto.org/rpc.html)):
interface references are capabilities — they both designate an object to
call and confer permission to call it. When a new object is created, only
the creator is initially able to call it. When the object is passed over
a network connection, the receiver gains permission to make calls — but
no one else does; others cannot access the capability without consent of
either the host or the receiver, because the host assigns it an ID
specific to the connection over which it was sent.

Consequences that matter for the estate:

- **No ambient authority, no ACL table to keep in sync.** Authority is
  the reference graph. A service never checks "who are you and what may
  you do" against a separate policy store for calls arriving on a
  capability; holding the (connection-scoped, unforgeable) reference
  _is_ the authorization. The spec doc notes this design avoids
  confused-deputy attacks while keeping legitimate use easy — and that
  ACLs can still be layered on top if wanted.
- **Attenuation is ordinary programming.** You attenuate by handing out a
  different object — a wrapper exposing fewer methods — not by minting a
  restricted token. Delegation is passing the reference on. There is no
  expiry field, no caveat language, no delegation-chain format: the model
  is _live objects_, not _bearer documents_.
- **Promise pipelining is the performance half of the same design.**
  Every call returns a promise whose interface-typed results can
  themselves be called (or passed as parameters) before the first call
  resolves; the client sends the whole chain in one flight and the server
  executes it as results land — you can chain any number of such calls
  together, with diamond dependencies and everything, and Cap'n Proto
  collapses them all into one round trip. This is what makes
  fine-grained object interfaces viable over high-latency links instead
  of forcing coarse singleton APIs.
- **Disconnect semantics are part of the model.** When a connection
  drops, every capability served over it becomes "disconnected" and
  calls throw; when the last reference to a server object is dropped,
  the object is closed/destroyed — servers get per-client resource
  cleanup without timeouts or leak-prone session tables.
- **Revocation is a pattern, not a primitive: the membrane.** Neither
  Cap'n Proto nor Cap'n Web has a "revoke this capability" verb. The
  ocap literature's answer, which this family expects you to build, is
  the **membrane**: a revocable forwarder that wraps the target _and
  transitively wraps every capability that flows through it in either
  direction_ (arguments and return values), so revoking the membrane
  revokes the whole subgraph of access that ever crossed it. A plain
  revocable forwarder leaks authority precisely because capabilities pass
  through it unwrapped; the membrane exists to fix that. Sandstorm is
  the worked example at platform scale: the platform tracks the
  capability graph between apps so access can be visualized and revoked
  by the user, and its "Powerbox" picker is capability introduction as a
  UX.

## The level model — spec ambition vs. shipped reality

The RPC spec divides features into levels an implementation advertises
([rpc.html, Protocol Features](https://capnproto.org/rpc.html)):

| Level | Feature                                | What it means                                                                                                                                          |
| ----- | -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 1     | Object references + promise pipelining | Two-party capability RPC as described above.                                                                                                           |
| 2     | Persistent capabilities                | "Save" a capability to a persistent token (a _sturdy ref_); "restore" it later, on a new connection. The host app must implement save/restore support. |
| 3     | Three-party handoff                    | If A hands B a reference to C, B and C automatically form a direct connection; calls stop proxying through A.                                          |
| 4     | Reference equality / joining           | Securely verify that capabilities received from different parties designate the same object (escrow-style patterns).                                   |

What implementations actually ship, per their own documentation:

- **C++ (reference implementation): Level 1.** Its own RPC page states
  that as of version 0.4 the C++ RPC implementation is a Level 1
  implementation, and persistent capabilities, three-way introductions,
  and distributed equality are not yet implemented
  ([capnproto.org/cxxrpc.html](https://capnproto.org/cxxrpc.html)). That
  status text is old, but an independent technology profile re-checked in
  October 2026 still records the C++ implementation as documenting only
  level 1. The C++ library does ship the _primitives_ for persistence
  (save/restore hooks) — Sandstorm built sturdy refs on them — but the
  restore semantics are the application's.
- **Rust (`capnp-rpc`): Level 1, explicitly.** "This is a level one
  implementation of the Cap'n Proto remote procedure call protocol. It
  is a fairly direct translation of the original C++ implementation"
  ([docs.rs/capnp-rpc README](https://docs.rs/crate/capnp-rpc/0.19.0/source/README.md)).
- **OCaml (`capnp-rpc`, by the Mirage/OCaml-CI folks): the furthest along
  — Level 2 complete, Level 3 not implemented.** With encryption and
  authentication using TLS and support for persistence; but messages
  between Carol and Bob are still routed via Alice. This is the
  exception that shows what the higher levels cost: a dedicated,
  security-focused team, years of work, one language.
- **Go** exposes a `Network` abstraction with `Introduce` /
  `DialIntroduced` and `ThirdPartyCapID` types — the level-3 handoff
  machinery exists in its API surface — but the package is versioned
  3.0.0-alpha, which tells its own maturity story.
- **Nobody ships Level 4.** And the direction of travel is away from
  standardizing the top levels: Kenton Varda argued on the project list
  in 2017 that level 2's sturdy-ref restore turned out to be
  intrinsically tied to the application (the Sandstorm lesson) and
  probably level 2 should be ripped out of the spec entirely.

**Bottom line for the estate:** when you adopt Cap'n Proto RPC you are
adopting _Level 1_: two-party, connection-scoped capabilities with
pipelining. Persistent capabilities, multi-vat handoff, and equality are
spec text and application-layer work, in every implementation you would
actually deploy. That is not a defect — Level 1 is the part that is
proven at Cloudflare scale — but any design that assumes "capnp gives us
distributed capabilities across a mesh" is assuming the unbuilt levels.

## Rust maturity

- The Rust implementation is a single-maintainer-lineage community
  project (David Renshaw, a Sandstorm core developer). Only the C++
  implementation is maintained by the core team; other languages "vary
  in readiness".
- It is alive and used: `capnp-rpc` shows 99 releases on lib.rs, roughly
  131k downloads/month, used in ~69 crates; the serialization crates
  track the same release train
  ([lib.rs/crates/capnp-rpc](https://lib.rs/crates/capnp-rpc)). Release
  cadence has historically been bursty, so this is a stable-but-thin
  maintenance bus factor, not a team.
- The RPC runtime is built on futures/tokio-adjacent plumbing, is
  transport-agnostic (its two-party VatNetwork runs over any
  bidirectional byte stream you hand it), and inherits the C++ design
  closely — which also means it inherits Level 1 as the ceiling.
- No WASM/Workers story for the Rust crates: on Cloudflare you would use
  Cap'n Proto via workerd's JS/Python RPC surface, not via `capnp-rpc`
  compiled to WASM (Workers' own runtime _is_ Cap'n Proto, but that is
  Cloudflare's C++ implementation, not the Rust crates).

## Encryption and identity — deliberately out of scope

- **Cap'n Proto specifies no encryption and no peer identity.** The spec
  says that at this time Cap'n Proto does not specify an encryption
  scheme, but as it is a simple byte stream protocol, it can easily be
  layered on top of SSL/TLS or other such protocols
  ([rpc.html, Encryption](https://capnproto.org/rpc.html)). A
  capability-based encrypted transport with three-party introductions was
  a roadmap aspiration, never a shipped spec.
- Therefore Cap'n Proto has **no identity model at all**: no keys, no
  names, no enrolment. The capability graph starts at a _bootstrap
  capability_ handed to whoever connects; authenticating _who_ connected
  (TLS client certs, a token, an iroh NodeId check) is entirely the
  embedding application's job. The one production-hardened pairing in the
  wild is OCaml's: TLS for encryption _and_ authentication under the
  capability layer.
- The C++ RPC layer also carries an explicit robustness caveat: per the
  project's FAQ, the RPC layer isn't yet robust against
  resource-exhaustion attacks — you rate-limit and bound in front of it.
- **Cap'n Web** is the same shape, web-flavoured: it runs over HTTPS/WSS
  (transport security is the platform's), and authentication is _in-band
  by design_ — the canonical pattern is an `authenticate(apiKey)` method
  that returns an authenticated session object you cannot forge, because
  the only way to hold one is to have it returned to you. Its own README
  security section is blunt about what stays yours: authenticating
  in-band rather than with cookies, rate-limiting because pipelining is
  cheap for attackers, setting transport payload limits, and validating
  types at runtime are the operator's responsibility.

## Cap'n Web in detail

- **What it is:** an object-capability RPC protocol + pure-TypeScript
  implementation, MIT-licensed, under 16 kB minified+gzipped with zero
  dependencies, running in all major browsers, Cloudflare Workers, Node,
  Bun and Deno ([cloudflare/capnweb](https://github.com/cloudflare/capnweb)).
  Repo created June 2025; 24 releases and ~4k GitHub stars at research
  time; the author and Cloudflare both call it new and experimental.
- **Wire format:** JSON with a pre/post-processing layer that encodes the
  special types JSON lacks (dates, bigints, byte arrays, errors, and —
  crucially — stubs and promises by reference). Human-readable on
  purpose; there is no schema and no code generation; TypeScript
  interfaces give compile-time checking only, and runtime validation is
  a separate concern.
- **Capability model:** same ocap core as Cap'n Proto — functions and
  `RpcTarget` objects pass _by reference_ as stubs; calls are
  bidirectional (a server can call a callback the client passed it);
  promise pipelining works in both WebSocket and HTTP-batch modes;
  stubs can be passed onward to other parties; disposal is explicit and
  reference-counted, with docs for session reconnection and what a
  disconnect destroys.
- **Transports:** HTTP (batch mode: one request/response carries a whole
  pipelined batch; references die when the batch completes), WebSocket
  (long-lived sessions, bidirectional), and `postMessage()`/MessagePort
  (iframe/worker boundaries) out of the box; custom transports are a
  documented extension point.
- **Workers integration:** it is the web-facing form of the Workers RPC
  system — a Worker can serve a Cap'n Web endpoint inside a normal
  `fetch` handler, and Cap'n Web stubs interoperate with Workers RPC
  stubs (service bindings, Durable Objects) per the README's interop
  docs. Workers RPC itself is production infrastructure at Cloudflare,
  built on Cap'n Proto.
- **What it deliberately leaves out:** identity/authentication (in-band
  `authenticate()` pattern only), encryption (HTTPS/WSS does it),
  persistence of capabilities across sessions, storage of any kind,
  group management, revocation (membrane pattern is yours to build), and
  any non-JS implementation (a Rust `capnweb-core` crate exists but is a
  single 0.1.0 release — a sketch, not a stack).

## The boundary — what neither provides (precise)

Neither Cap'n Proto nor Cap'n Web provides, at any shipped level:

1. **Identity / key management** — no keys, no names, no enrolment, no
   rotation.
2. **Authentication of peers** — the bootstrap capability goes to whoever
   holds the connection; who that is, is your problem.
3. **Encryption** — unspecified by Cap'n Proto; platform-provided for
   Cap'n Web. No forward-secrecy story of their own.
4. **Offline / persistent authority (as shipped)** — capabilities are
   connection-scoped; level-2 persistence is unimplemented in C++ and
   Rust and application-defined where it exists (OCaml). A capability
   cannot be exercised while disconnected, and cannot be carried in a
   stored document the way a signed token can.
5. **Token-style attenuation with expiry/caveats** — attenuation is by
   facet object, delegation by reference passing; there is no
   serializable, expiring, caveat-carrying credential format.
6. **Revocation as a primitive** — membranes/caretakers are patterns you
   implement; disconnection revokes incidentally.
7. **Group management** — no groups, membership, or roles exist in the
   model; a "group" is an object you build whose methods hand out facets.
8. **Sync / replication** — nothing. RPC is live request/response between
   connected parties; there is no log, no CRDT, no store-and-forward, no
   merge semantics. Offline behaviour is "calls throw disconnected."
9. **Storage** — nothing. Messages can be written to disk (the
   serialization format is mmap-friendly), but there is no store, index,
   or query layer, and no SQLite/Turso integration story.
10. **Discovery / NAT traversal / rendezvous** — a Cap'n Proto vat must
    already have a byte stream to its peer. Level 3 would have formed
    direct connections after introduction; it is unimplemented in the
    implementations above, so introduction today means proxying through
    the introducer.

## Axes summary

Scores 1 (absent/poor) – 5 (excellent), estate-weighted.

| Axis                       | Cap'n Proto (Rust) | Cap'n Web | Note                                                                                                                                                  |
| -------------------------- | ------------------ | --------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| Identity & key management  | 1                  | 1         | None in either; bootstrap-cap + your authn.                                                                                                           |
| Capability model           | 4                  | 4         | True ocap with pipelining; docked for connection-scoped lifetime and pattern-only revocation.                                                         |
| — delegation               | 4                  | 4         | Reference passing; Cap'n Web stubs pass onward between parties.                                                                                       |
| — attenuation              | 4                  | 4         | Facet objects; no caveat language needed or available.                                                                                                |
| — revocation               | 2                  | 2         | Membrane pattern only; disconnect revokes incidentally.                                                                                               |
| — expiry                   | 1                  | 1         | No expiry concept; session/batch lifetime is the bound.                                                                                               |
| Group management           | 1                  | 1         | Build-it-yourself objects.                                                                                                                            |
| Encryption                 | 2                  | 3         | Proto: bring TLS/Noise yourself. Web: HTTPS/WSS platform default.                                                                                     |
| Transport & NAT            | 2                  | 3         | Proto: any byte stream, no rendezvous/NAT help. Web: HTTP/WS/postMessage — excellent on the web, no p2p NAT story.                                    |
| Sync/replication           | 1                  | 1         | None; live RPC only.                                                                                                                                  |
| Storage fit (SQLite/Turso) | 2                  | 1         | Proto serialization is a fine signable blob format for rows/blobs; no store integration. Web: JSON only.                                              |
| Rust maturity              | 3                  | 1         | `capnp`/`capnp-rpc` real, Level 1, thin maintenance. Cap'n Web in Rust: one 0.1.0 crate.                                                              |
| Cloudflare fit             | 5                  | 5         | workerd _is_ Cap'n Proto; Cap'n Web is a first-party Workers citizen.                                                                                 |
| iroh fit                   | 3                  | 2         | Proto runs happily _over_ an iroh stream and iroh supplies the identity/encryption capnp lacks — composable, not native. Web: no iroh story.          |
| Spec/interop status        | 3                  | 2         | Proto: stable spec, levels mostly unimplemented, multi-language serialization interop is real. Web: single-vendor spec, pre-1.0, JS-only in practice. |
| Project health             | 4                  | 3         | Proto: Cloudflare-maintained, production at huge scale. Web: active and first-party but months old and self-described experimental.                   |
| Operational complexity     | 3                  | 4         | Proto: schema discipline + you build authn/transport/persistence. Web: near-zero setup, but you build everything above RPC.                           |
| Offline/local-first        | 1                  | 1         | Disconnected = broken capabilities, by design.                                                                                                        |

**Verdict.** Cap'n Proto's real advantage is narrow and deep: it is the
only candidate in this whole investigation that gives you _live,
unforgeable, delegable authority as a programming model_ — with promise
pipelining making that model fast enough to use naively — and it is
proven inside the exact edge runtime this estate already deploys
(Workers). Cap'n Web extends the same advantage to browsers and Worker
HTTP surfaces with almost zero ceremony. But both are **call-layer
technologies, not auth or sync foundations**: identity, persistence of
authority, offline operation, groups, revocation-as-a-system, storage
and replication are all outside the boundary. In a composed stack,
capnp/capnweb sit _above_ a transport that gives them identity and
encryption (iroh streams; TLS/WSS at the edge) and _beside_ whatever
provides durable, offline-checkable authority (tokens/documents) and
replication. They do not compete with p2panda-auth/sync so much as occupy
a layer those systems don't address: what an already-authorized live
session can do, call by call.
