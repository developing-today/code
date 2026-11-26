# Cap'n Proto / Cap'n Web in depth, and the wider field survey

Research lane CAPNP-SURVEY for the auth + transport investigation program
(tracking issue: developing-today/code#13779). Master prompt:
`~/workspace/p2panda-investigation/p2panda-program-investigation-prompt.md`.
All claims are from primary sources (project docs, specs, repos, crates.io)
and linked inline. Estate shorthand used below: Rust-first core work,
SQLite/Turso stores, Cloudflare Workers/KV/R2 at the edge, iroh (ed25519
NodeId identity, relays + hole punching) as the chosen p2p transport
direction, an existing fleet CA program (Ed25519 / P-256 / RSA-4096 /
ML-DSA-44 roots) and an interim IdP CA (EC P-384) feeding a minimal OIDC
provider.

---

# Part 1 — Cap'n Proto and Cap'n Web

## 1.1 What the two pieces are

**Cap'n Proto** is two things sharing a schema language: (a) a binary
**serialization format** whose wire layout is the in-memory layout — fixed
widths, fixed offsets, offset-based pointers, little-endian — so a message
needs _no_ encode/decode pass and can be mmap'd, randomly accessed, and read
incrementally; and (b) an **RPC system** in which interface references are a
first-class type in the schema, so a method parameter or return value can be
a live reference to a remote object
([capnproto.org introduction](https://capnproto.org/),
[RPC protocol doc](https://capnproto.org/rpc.html)). It was written by Kenton
Varda (primary author of Protocol Buffers v2) and is now maintained by the
Cloudflare Workers team; it is the RPC substrate inside `workerd` and was
built for Sandstorm.io
([capnproto.org](https://capnproto.org/),
[cloudparts.dev technology profile, facts checked 2026-10](https://cloudparts.dev/technologies/capnproto/)).

**Cap'n Web** is a separate, younger protocol by the same author: a
schema-free, JSON-based object-capability RPC implementation in pure
TypeScript for browsers and web servers, announced by Cloudflare in
September 2025 and explicitly "experimental"
([Cloudflare blog: Cap'n Web](https://blog.cloudflare.com/capnweb-javascript-rpc-library/)).
It is _not_ wire-compatible with Cap'n Proto; it is a sibling design that
borrows the capability model and promise pipelining and drops the schema and
the binary format. It interoperates with Workers RPC (which is itself Cap'n
Proto under the hood) through workerd
([cloudflare/capnweb README](https://github.com/cloudflare/capnweb),
[Cloudflare blog: JavaScript-native RPC](https://blog.cloudflare.com/javascript-native-rpc/)).

## 1.2 The serialization layer (brief, because it is not the decision)

- Zero-copy: "there is no encoding/decoding step. The Cap'n Proto encoding is
  appropriate both as a data interchange format and an in-memory
  representation" ([capnproto.org](https://capnproto.org/)). A "packing"
  codec strips zero bytes to reach protobuf-class wire sizes while staying
  faster than a parse-based format.
- Schema evolution is structural: fields are numbered in the order added,
  new fields go at the end, readers bounds-check — old and new schemas
  interoperate without a version negotiation
  ([capnproto.org](https://capnproto.org/)). Struct-list canonicalization
  exists specifically "to make Cap'n Proto more appropriate for use in
  cryptographic protocols" — i.e. a canonical byte form you can sign
  ([0.5 release notes](https://capnproto.org/news/2014-12-15-capnproto-0.5-generics-msvc-java-csharp.html)).
- Accessors validate pointers before following them; the project has had
  fuzzing and expert security review
  ([capnproto.org](https://capnproto.org/)).

Estate relevance: Cap'n Proto serialization is a fine fit anywhere the
estate wants a fast, signable, schema-evolving binary message format, and it
is _already_ the format under Workers RPC. But serialization is not what
this investigation is deciding; the RPC capability model is.

## 1.3 The RPC capability model

Cap'n Proto RPC is a distributed-object protocol descended from CapTP (the E
language's protocol). The core rule, from the spec doc:

> "Cap'n Proto interface references are capabilities. That is, they both
> designate an object to call and confer permission to call it. When a new
> object is created, only the creator is initially able to call it. When the
> object is passed over a network connection, the receiver gains permission
> to make calls – but no one else does. In fact, it is impossible for others
> to access the capability without consent of either the host or the receiver
> because the host only assigns it an ID specific to the connection over
> which it was sent."
> ([capnproto.org/rpc.html, Security](https://capnproto.org/rpc.html))

Consequences that matter for the estate:

- **No ambient authority, no ACL table to keep in sync.** Authority is the
  reference graph. A service never checks "who are you and what may you do"
  against a separate policy store for calls arriving on a capability; holding
  the (connection-scoped, unforgeable) reference _is_ the authorization. The
  spec doc notes this design "avoid[s] confused-deputy attacks" while
  keeping legitimate use easy — and that ACLs can still be layered on top if
  wanted ([rpc.html](https://capnproto.org/rpc.html)).
- **Attenuation is ordinary programming.** You attenuate by handing out a
  different object — a wrapper exposing fewer methods — not by minting a
  restricted token. Delegation is passing the reference on. There is no
  expiry field, no caveat language, no delegation-chain format: the model is
  _live objects_, not _bearer documents_.
- **Promise pipelining is the performance half of the same design.** Every
  call returns a promise whose interface-typed results can themselves be
  called (or passed as parameters) before the first call resolves; the
  client sends the whole chain in one flight and the server executes it as
  results land — "you can chain any number of such calls together – with
  diamond dependencies and everything – and Cap'n Proto will collapse them
  all into one round trip" ([rpc.html](https://capnproto.org/rpc.html)).
  This is what makes fine-grained object interfaces viable over
  high-latency links instead of forcing coarse singleton APIs.
- **Disconnect semantics are part of the model.** When a connection drops,
  every capability served over it becomes "disconnected" and calls throw;
  when the last reference to a server object is dropped, the object is
  closed/destroyed — servers get per-client resource cleanup without
  timeouts or leak-prone session tables
  ([rpc.html](https://capnproto.org/rpc.html)).
- **Revocation is a pattern, not a primitive: the membrane.** Neither Cap'n
  Proto nor Cap'n Web has a "revoke this capability" verb. The ocap
  literature's answer, which this family expects you to build, is the
  **membrane**: a revocable forwarder that wraps the target _and
  transitively wraps every capability that flows through it in either
  direction_ (arguments and return values), so revoking the membrane
  revokes the whole subgraph of access that ever crossed it. A plain
  revocable forwarder leaks authority precisely because capabilities pass
  through it unwrapped; the membrane exists to fix that
  ([ERights wiki: Capability Patterns — Membranes](http://wiki.erights.org/mediawiki/index.php?title=Walnut/Secure_Distributed_Computing/Capability_Patterns&diff=cur&oldid=2736&printable=yes&printable=yes);
  [E FAQ §2.6: "Uncooperative revocability requires the Membrane
  pattern"](https://mumble.net/e/faq.html)). Sandstorm is the worked
  example at platform scale: the platform tracks the capability graph
  between apps so access can be visualized and revoked by the user, and its
  "Powerbox" picker is capability introduction as a UX
  ([0.5 release notes](https://capnproto.org/news/2014-12-15-capnproto-0.5-generics-msvc-java-csharp.html)).

## 1.4 The level model — spec ambition vs. shipped reality

The RPC spec divides features into levels an implementation advertises
([rpc.html, Protocol Features](https://capnproto.org/rpc.html)):

| Level | Feature                                | What it means                                                                                                                                          |
| ----- | -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 1     | Object references + promise pipelining | Two-party capability RPC as described above.                                                                                                           |
| 2     | Persistent capabilities                | "Save" a capability to a persistent token (a _sturdy ref_); "restore" it later, on a new connection. The host app must implement save/restore support. |
| 3     | Three-party handoff                    | If A hands B a reference to C, B and C automatically form a direct connection; calls stop proxying through A.                                          |
| 4     | Reference equality / joining           | Securely verify that capabilities received from different parties designate the same object (escrow-style patterns).                                   |

What implementations actually ship, per their own documentation:

- **C++ (reference implementation): Level 1.** Its own RPC page states:
  "As of version 0.4, Cap'n Proto's C++ RPC implementation is a Level 1
  implementation. Persistent capabilities, three-way introductions, and
  distributed equality are not yet implemented"
  ([capnproto.org/cxxrpc.html](https://capnproto.org/cxxrpc.html)). That
  status text is old, but an independent technology profile re-checked in
  October 2026 still records the C++ implementation as documenting only
  level 1 ([cloudparts.dev](https://cloudparts.dev/technologies/capnproto/)).
  The C++ library does ship the _primitives_ for persistence
  (`persistent.capnp` save/restore hooks, added in 0.5) — Sandstorm built
  sturdy refs on them — but the restore semantics are the application's
  ([0.5 release notes](https://capnproto.org/news/2014-12-15-capnproto-0.5-generics-msvc-java-csharp.html)).
- **Rust (`capnp-rpc`): Level 1, explicitly.** "This is a level one
  implementation of the Cap'n Proto remote procedure call protocol. It is a
  fairly direct translation of the original C++ implementation"
  ([docs.rs/capnp-rpc README](https://docs.rs/crate/capnp-rpc/0.19.0/source/README.md);
  docs.rs crate page: "Includes all Level 1 features"
  ([docs.rs/capnp_rpc](https://docs.rs/capnp-rpc/latest/capnp_rpc/))).
- **OCaml (`capnp-rpc`, by the Mirage/OCaml-CI folks): the furthest along —
  Level 2 complete, Level 3 not implemented.** "RPC Level 2 is complete,
  with encryption and authentication using TLS and support for persistence…
  Level 3 support is not implemented yet, so… messages between Carol and
  Bob will be routed via Alice"
  ([ocaml.org capnp-rpc 2.1](https://ocaml.org/p/capnp-rpc/2.1)). This is the
  exception that shows what the higher levels cost: a dedicated,
  security-focused team, years of work, one language.
- **Go (`capnproto.org/go/capnp/v3`)** exposes a `Network` abstraction with
  `Introduce` / `DialIntroduced` and `ThirdPartyCapID` types — the
  level-3 handoff machinery exists in its API surface
  ([pkg.go.dev rpc package](https://pkg.go.dev/capnproto.org/go/capnp/v3@v3.0.0-alpha-29/rpc)),
  but the package is versioned 3.0.0-alpha, which tells its own maturity
  story.
- **Nobody ships Level 4.** And the direction of travel is away from
  standardizing the top levels: Kenton Varda argued on the project list in
  2017 that level 2's sturdy-ref restore turned out to be "intrinsically
  tied to" the application (the Sandstorm lesson) and "probably level 2
  should be ripped out of the spec entirely"
  ([capnproto list archive, 2017](https://www.mail-archive.com/capnproto@googlegroups.com/msg00612.html)).

**Bottom line for the estate:** when you adopt Cap'n Proto RPC you are
adopting _Level 1_: two-party, connection-scoped capabilities with
pipelining. Persistent capabilities, multi-vat handoff, and equality are
spec text and application-layer work, in every implementation you would
actually deploy. That is not a defect — Level 1 is the part that is proven
at Cloudflare scale — but any design that assumes "capnp gives us
distributed capabilities across a mesh" is assuming the unbuilt levels.

## 1.5 Rust maturity (`capnp`, `capnpc`, `capnp-rpc`)

- The Rust implementation is a single-maintainer-lineage community project
  (David Renshaw, `@dwrensha`, a Sandstorm core developer), listed by the
  project among its serialization+RPC implementations
  ([capnproto.org/otherlang.html](https://capnproto.org/otherlang.html)).
  Only the C++ implementation is maintained by the core team; other
  languages "vary in readiness"
  ([cloudparts.dev](https://cloudparts.dev/technologies/capnproto/)).
- It is alive and used: `capnp-rpc` shows 99 releases on lib.rs, roughly
  131k downloads/month, used in ~69 crates; the serialization crates track
  the same release train
  ([lib.rs/crates/capnp-rpc](https://lib.rs/crates/capnp-rpc)). Release
  cadence has historically been bursty (the 0.15 release post notes "almost
  two years" since 0.14), so this is a stable-but-thin maintenance bus
  factor, not a team
  ([dwrensha release notes](https://dwrensha.github.io/capnproto-rust/2022/11/03/0.15-release.html)).
- The RPC runtime is built on futures/`tokio`-adjacent plumbing
  (`capnp-futures`), is transport-agnostic (its `twoparty::VatNetwork`
  runs over any bidirectional byte stream you hand it), and inherits the
  C++ design closely — which also means it inherits Level 1 as the ceiling
  ([docs.rs/capnp_rpc](https://docs.rs/capnp-rpc/latest/capnp_rpc/)).
- No WASM/Workers story for the Rust crates: on Cloudflare you would use
  Cap'n Proto via workerd's JS/Python RPC surface, not via `capnp-rpc`
  compiled to WASM (Workers' own runtime _is_ Cap'n Proto, but that is
  Cloudflare's C++ implementation, not the Rust crates)
  ([Cloudflare blog: JavaScript-native RPC](https://blog.cloudflare.com/javascript-native-rpc/)).

## 1.6 Encryption and identity — deliberately out of scope

- **Cap'n Proto specifies no encryption and no peer identity.** "At this
  time, Cap'n Proto does not specify an encryption scheme, but as it is a
  simple byte stream protocol, it can easily be layered on top of SSL/TLS or
  other such protocols" ([rpc.html, Encryption](https://capnproto.org/rpc.html)).
  A capability-based encrypted transport with three-party introductions was
  a roadmap aspiration, never a shipped spec
  ([2017 list discussion quoting the roadmap](https://www.mail-archive.com/capnproto@googlegroups.com/msg00612.html)).
- Therefore Cap'n Proto has **no identity model at all**: no keys, no
  names, no enrolment. The capability graph starts at a _bootstrap
  capability_ handed to whoever connects; authenticating _who_ connected
  (TLS client certs, a token, an iroh NodeId check) is entirely the
  embedding application's job. The one production-hardened pairing in the
  wild is OCaml's: TLS for encryption _and_ authentication under the
  capability layer ([ocaml.org](https://ocaml.org/p/capnp-rpc/2.1)).
- The C++ RPC layer also carries an explicit robustness caveat: per the
  project's FAQ as summarized by the cloudparts profile, the RPC layer
  "isn't yet robust against resource-exhaustion attacks" — you rate-limit
  and bound in front of it
  ([cloudparts.dev](https://cloudparts.dev/technologies/capnproto/)).
- **Cap'n Web** is the same shape, web-flavoured: it runs over HTTPS/WSS
  (transport security is the platform's), and authentication is _in-band by
  design_ — the canonical pattern is an `authenticate(apiKey)` method that
  returns an authenticated session object you cannot forge, because the
  only way to hold one is to have it returned to you
  ([Cloudflare blog: Cap'n Web, "Did you spot the security?"](https://blog.cloudflare.com/capnweb-javascript-rpc-library/)).
  Its own README security section is blunt about what stays yours:
  "authenticating in-band rather than with cookies, rate-limiting because
  pipelining is cheap for attackers, setting transport payload limits, and
  validating types at runtime" are the operator's responsibility
  ([cloudflare/capnweb README, Security](https://github.com/cloudflare/capnweb)).

## 1.7 Cap'n Web in detail

- **What it is:** an object-capability RPC protocol + pure-TypeScript
  implementation, MIT-licensed, under 16 kB minified+gzipped with zero
  dependencies, running in all major browsers, Cloudflare Workers, Node,
  Bun and Deno ([cloudflare/capnweb](https://github.com/cloudflare/capnweb)).
  Repo created June 2025; 24 releases and ~4k GitHub stars at research
  time; the author and Cloudflare both call it new and experimental
  ([repo metadata](https://github.com/cloudflare/capnweb),
  [blog](https://blog.cloudflare.com/capnweb-javascript-rpc-library/)).
- **Wire format:** JSON with a pre/post-processing layer that encodes the
  special types JSON lacks (dates, bigints, byte arrays, errors, and —
  crucially — stubs and promises by reference). Human-readable on purpose;
  there is no schema and no code generation; TypeScript interfaces give
  compile-time checking only, and runtime validation is a separate concern
  (the docs point at Zod-style validation and a `capnweb-validate`
  companion package)
  ([blog](https://blog.cloudflare.com/capnweb-javascript-rpc-library/),
  [README](https://github.com/cloudflare/capnweb)).
- **Capability model:** same ocap core as Cap'n Proto — functions and
  `RpcTarget` objects pass _by reference_ as stubs; calls are bidirectional
  (a server can call a callback the client passed it); promise pipelining
  works in both WebSocket and HTTP-batch modes, including the signature
  `.map()` over pipelined promise arrays; stubs can be passed onward to
  other parties; disposal is explicit and reference-counted (`using`
  declarations, `Symbol.dispose`), with docs for session reconnection and
  what a disconnect destroys
  ([README](https://github.com/cloudflare/capnweb),
  [blog](https://blog.cloudflare.com/capnweb-javascript-rpc-library/)).
- **Transports:** HTTP (batch mode: one request/response carries a whole
  pipelined batch; references die when the batch completes), WebSocket
  (long-lived sessions, bidirectional), and `postMessage()`/MessagePort
  (iframe/worker boundaries) out of the box; custom transports are a
  documented extension point
  ([README](https://github.com/cloudflare/capnweb),
  [blog](https://blog.cloudflare.com/capnweb-javascript-rpc-library/)).
- **Workers integration:** it is the web-facing form of the Workers RPC
  system — a Worker can serve a Cap'n Web endpoint with
  `newWorkersRpcResponse(request, new MyApiServer())` inside a normal
  `fetch` handler, and Cap'n Web stubs interoperate with Workers RPC stubs
  (service bindings, Durable Objects) per the README's interop docs
  ([README](https://github.com/cloudflare/capnweb),
  [blog: JavaScript-native RPC](https://blog.cloudflare.com/javascript-native-rpc/)).
  Workers RPC itself is production infrastructure at Cloudflare, built on
  Cap'n Proto, with Worker-to-Worker calls often never leaving the thread
  ([blog](https://blog.cloudflare.com/javascript-native-rpc/)).
- **What it deliberately leaves out:** identity/authentication (in-band
  `authenticate()` pattern only — §1.6), encryption (HTTPS/WSS does it),
  persistence of capabilities across sessions (sessions/resume are
  documented, but there is no sturdy-ref/store story), storage of any
  kind, group management, revocation (membrane pattern is yours to
  build), and any non-JS implementation (a Rust `capnweb-core` crate
  exists but is a single 0.1.0 release — a sketch, not a stack
  ([lib.rs/crates/capnweb-core](https://lib.rs/crates/capnweb-core))).

## 1.8 The boundary — what neither provides (precise)

Neither Cap'n Proto nor Cap'n Web provides, at any shipped level:

1. **Identity / key management** — no keys, no names, no enrolment, no
   rotation. (§1.6)
2. **Authentication of peers** — the bootstrap capability goes to whoever
   holds the connection; who that is, is your problem. (§1.6)
3. **Encryption** — unspecified by Cap'n Proto; platform-provided for
   Cap'n Web. No forward-secrecy story of their own. (§1.6)
4. **Offline / persistent authority (as shipped)** — capabilities are
   connection-scoped; level-2 persistence is unimplemented in C++ and Rust
   and application-defined where it exists (OCaml). There is no offline
   delegation: a capability cannot be exercised while disconnected, and
   cannot be carried in a stored document the way a signed token can.
5. **Token-style attenuation with expiry/caveats** — attenuation is by
   facet object, delegation by reference passing; there is no serializable,
   expiring, caveat-carrying credential format.
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
    the introducer (OCaml's documented behaviour,
    [ocaml.org](https://ocaml.org/p/capnp-rpc/2.1)).

## 1.9 Axes summary — Part 1

Scores 1 (absent/poor) – 5 (excellent), estate-weighted.

| Axis                       | Cap'n Proto (Rust) | Cap'n Web | Note                                                                                                                                                                 |
| -------------------------- | ------------------ | --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Identity & key management  | 1                  | 1         | None in either; bootstrap-cap + your authn.                                                                                                                          |
| Capability model           | 4                  | 4         | True ocap with pipelining; docked for connection-scoped lifetime and pattern-only revocation.                                                                        |
| — delegation               | 4                  | 4         | Reference passing; Cap'n Web stubs pass onward between parties.                                                                                                      |
| — attenuation              | 4                  | 4         | Facet objects; no caveat language needed or available.                                                                                                               |
| — revocation               | 2                  | 2         | Membrane pattern only; disconnect revokes incidentally.                                                                                                              |
| — expiry                   | 1                  | 1         | No expiry concept; session/batch lifetime is the bound.                                                                                                              |
| Group management           | 1                  | 1         | Build-it-yourself objects.                                                                                                                                           |
| Encryption                 | 2                  | 3         | Proto: bring TLS/Noise yourself. Web: HTTPS/WSS platform default. Neither has an own story or FS beyond the channel's.                                               |
| Transport & NAT            | 2                  | 3         | Proto: any byte stream, no rendezvous/NAT help. Web: HTTP/WS/postMessage — excellent on the web, no p2p NAT story.                                                   |
| Sync/replication           | 1                  | 1         | None; live RPC only.                                                                                                                                                 |
| Storage fit (SQLite/Turso) | 2                  | 1         | Proto serialization is a fine signable blob format for rows/blobs; no store integration. Web: JSON only.                                                             |
| Rust maturity              | 3                  | 1         | `capnp`/`capnp-rpc` real, Level 1, thin maintenance. Cap'n Web in Rust: one 0.1.0 crate.                                                                             |
| Cloudflare fit             | 5                  | 5         | workerd _is_ Cap'n Proto; Cap'n Web is a first-party Workers citizen.                                                                                                |
| iroh fit                   | 3                  | 2         | Proto runs happily _over_ an iroh stream (byte-stream transport) and iroh supplies the identity/encryption capnp lacks — composable, not native. Web: no iroh story. |
| Spec/interop status        | 3                  | 2         | Proto: stable spec, levels mostly unimplemented, multi-language serialization interop is real. Web: single-vendor spec, pre-1.0, JS-only in practice.                |
| Project health             | 4                  | 3         | Proto: Cloudflare-maintained, LTS 1.0 (2023) + 2.0 branch, production at huge scale. Web: active and first-party but months old and self-described experimental.     |
| Operational complexity     | 3                  | 4         | Proto: schema discipline + you build authn/transport/persistence. Web: near-zero setup, but you build everything above RPC.                                          |
| Offline/local-first        | 1                  | 1         | Disconnected = broken capabilities, by design.                                                                                                                       |

**Part 1 verdict.** Cap'n Proto's real advantage is narrow and deep: it is
the only candidate in this whole investigation that gives you _live,
unforgeable, delegable authority as a programming model_ — with promise
pipelining making that model fast enough to use naively — and it is proven
inside the exact edge runtime this estate already deploys (Workers). Cap'n
Web extends the same advantage to browsers and Worker HTTP surfaces with
almost zero ceremony. But both are **call-layer technologies, not auth or
sync foundations**: identity, persistence of authority, offline operation,
groups, revocation-as-a-system, storage and replication are all outside
the boundary (§1.8). In a composed stack, capnp/capnweb sit _above_ a
transport that gives them identity and encryption (iroh streams; TLS/WSS
at the edge) and _beside_ whatever provides durable, offline-checkable
authority (tokens/documents) and replication. They do not compete with
p2panda-auth/sync so much as occupy a layer those systems don't address:
what an already-authorized live session can do, call by call.

---

# Part 2 — The wider field, briefly

One tight entry per system: what it is, the capability/identity story, the
estate fit, and scores on the axes this estate weights most (capability
model depth · Rust maturity · Cloudflare/WASM fit · spec stability ·
project health, plus transport/sync where relevant). Flags (⚑) mark entries
that plausibly **beat the named families** (p2panda, keyhive/subduction,
willow/meadowcap, the ocap network family, capnp/capnweb) on an axis this
estate cares about — those are the ones synthesis should deep-dive.

## 2.1 ⚑ UCAN — the deepest capability-token model; Rust 1.0 not ready

UCAN (User Controlled Authorization Networks) is a trustless, local-first
delegation system in the SPKI/SDSI lineage: chains of signed delegations
(`iss → aud`, about a `sub`) presented as proofs at invocation, maintained
by the UCAN Working Group with Storacha (Protocol Labs spin-out) as the
main production lineage
([ucan-wg/spec](https://github.com/ucan-wg/spec/)). Its capability model is
the richest of any token system surveyed: delegation is a required
sub-spec; attenuation is _mandatory monotonic narrowing_ expressed through
hierarchical commands (`/crud` proves `/crud/read`) plus a policy language
over invocation arguments; **invocation is a separate signed object from
delegation** (exercising authority ≠ holding it — unique here); revocation
exists as a reserved-namespace invocation that kills a delegation CID and
all derivatives, though _how a validator learns of a revocation is
explicitly out of scope_ — the practical weak point. The 1.0 spec is a
breaking change most secondary sources miss: ≤0.10 was JWT, but **1.0 is
canonical DAG-CBOR in a Varsig envelope, content-addressed by CID, with
`did:key` required and Ed25519 preferred** (P-256/secp256k1 also in suite;
**P-384 is not**, so the interim IdP CA's curve is a non-interoperable
extension here). The `did:key` principal is mechanically an iroh NodeId in
multicodec dress — the identity fit with this estate is exact. What blocks
adoption today is Rust: the WG's `rs-ucan` still implements the pre-1.0 JWT
spec (0.10 canary), and conformant 1.0 Rust exists only as third-party
pre-release crates. UCAN pointedly does not do transport, storage, groups,
or discovery. Sources:
[spec](https://github.com/ucan-wg/spec/) ·
[go-ucan token docs](https://pkg.go.dev/github.com/ucan-wg/go-ucan/token/delegation) ·
[rs-ucan status via WG staging docs](https://8a810bd9.ucan-staging.pages.dev/libraries/rust/) ·
[pq-ucan (third-party 1.0 Rust)](https://docs.rs/crate/pq-ucan/latest)

Scores: capability depth **5** · Rust maturity **2** · Cloudflare/WASM fit
**3** · spec stability **3** (1.0.0 on main, untagged; JWT→CBOR break
orphaned 0.10) · project health **3** (active WG + Storacha production;
small, grant-paced).
**⚑ Flag:** beats every named family on _token-model semantics_ —
delegation + invocation + policy + revocation in one spec, with an
identity form (`did:key`/Ed25519) that is literally the iroh NodeId.
Deep-dive in synthesis as the semantic reference for offline authority,
even if the Rust implementation is home-grown or deferred.

## 2.2 ⚑ Biscuit — the adoptable offline token for this estate, today

Biscuit (now an Eclipse Foundation project, ex-Clever Cloud) is a bearer
authorization token combining Macaroons-style offline attenuation with
public-key verification and a **Datalog-derived rights language**,
serialized as Protobuf
([spec](https://doc.biscuitsec.org/reference/specifications)). The token is
an append-only chain of signed blocks: block 0 carries the authority's
rights as Datalog facts/rules; each holder appends blocks using chained
ephemeral keys, and attenuation is expressed as _checks_ that must all
succeed at verification; scoping rules prevent later blocks from widening
rights. Third-party blocks (v3.2+, externally signed, with Datalog able to
require facts `trusting ed25519/<key>`) are Macaroons' discharge redesigned
for public keys — also its historically fragile area (CVE-2024-42350,
fixed by a signature-payload change). Revocation is deliberately external:
the crate generates a per-block `revocation_id` and the verifier keeps a
deny-list — revoking a block kills everything attenuated from it, and the
list is a trivial SQLite table in this estate. Expiry is idiomatic (a
`time` check), not a built-in field. Identity is out of scope (the docs
state plainly it is _not an authentication protocol_); keys are Ed25519 by
default, P-256 supported, P-384 not. The estate fit is the best of the
token family: **Rust is the reference implementation** (`biscuit-auth`
6.0.0, 2025-07), with parser/macro/CLI/C-API crates in-org, and an
**official WASM build** (`biscuit-wasm`) that makes Workers verification
realistic today (modulo wrangler bundling of the `.wasm` asset). Sources:
[spec](https://doc.biscuitsec.org/reference/specifications) ·
[biscuit-auth on lib.rs](https://lib.rs/crates/biscuit-auth) ·
[Rust usage docs](https://doc.biscuitsec.org/usage/rust.html) ·
[biscuit-wasm 0.4.0](https://www.biscuitsec.org/blog/biscuit-wasm-0-4-0/)

Scores: capability depth **4** (most expressive check language here;
docked for bearer-only delegation with no invocation semantics) · Rust
maturity **5** · Cloudflare/WASM fit **5** · spec stability **4**
(versioned, additive-first 3.x, conformance fixtures; two breaking
third-party-block security fixes) · project health **4** (Eclipse
governance, production at Clever Cloud/Outscale/Pulsar; two-person core).
**⚑ Flag:** beats the named families on the _deployable offline-token_
axis — it is the only candidate in the whole survey that is simultaneously
Rust-reference-implemented, WASM-official, attenuation-native, and
public-key verifiable. If the estate needs expiring, attenuable,
offline-checkable authority documents (the thing capnp pointedly lacks,
§1.8 item 5), Biscuit is the off-the-shelf answer; NodeId binding and the
revocation list are thin house layers on top.

## 2.3 Macaroons — prior art, not a dependency

Macaroons (Google et al., NDSS 2014) introduced contextual caveats: a
bearer credential built from chained HMACs, where anyone holding the token
can append caveats (first-party predicates checked by the target;
third-party caveats discharged by a named third party, with the discharge
bound to the authorizing macaroon to prevent replay)
([paper](https://research.google/pubs/macaroons-cookies-with-contextual-caveats-for-decentralized-authorization-in-the-cloud/)).
Both of its ideas won — offline monotonic attenuation lives on in Biscuit,
discharge in Biscuit's third-party blocks and UCAN's design — but the
construction lost for this estate: verification is **symmetric-only**, so
every verifier must hold the minting secret (and can therefore mint),
which is architecturally opposite to a public-key/CA estate; there is no
spec home, no maintainer, no caveat vocabulary standard, no revocation
(rely on short lifetimes); and the namesake Rust crate self-describes as
not-for-production and has been unmaintained since 2022
([macaroon crate source](https://docs.rs/crate/macaroon/0.1.1/source/src/lib.rs),
[lib.rs](https://lib.rs/crates/macaroon)). If a macaroon-shaped token were
ever needed inside a Worker, the honest route is ~100 lines of WebCrypto
HMAC, not a crate.

Scores: capability depth **3** · Rust maturity **2** · Cloudflare/WASM fit
**2** · spec stability **2** (frozen by abandonment; V1/V2/V2J folklore) ·
project health **2**.

## 2.4 Veilid — a second network, not a layer

Veilid is a mobile-first p2p _application framework_ from the Cult of the
Dead Cow / Veilid Foundation: Rust-first (`veilid-core`, active 0.5.x line),
with its own transport set (UDP/TCP/WS/WSS, own hole-punching and relays),
ed25519 node identity, and a differentiator this estate has not asked for —
**privacy routing** (safety/private routes mask sender and receiver IPs and
NodeIds), plus a DHT key–multi-value store with watches
([veilid.com FAQ](https://veilid.com/faq/),
[veilid-core docs](https://docs.rs/crate/veilid-core/latest)). The DHT is
storage, not sync: no log, no CRDT, no merge semantics; conflict resolution
is the app's problem. Auth is thin: DHT record ownership (owner keypair =
write capability for subkeys) and route blobs as bearer capabilities; no
scoped, delegatable, revocable capability system. Maturity is pre-1.0 with
breaking changes, a beta flagship chat app, and no verified large-scale
third-party production deployment. It _competes_ with iroh — incompatible
protocol, identity format, and network — and its one thing iroh lacks (the
DHT) cannot be taken without its transport.

Scores: transport/NAT **3** · Rust maturity **3** · sync semantics **2** ·
project health **3** · iroh-estate fit **2**.

## 2.5 Holepunch / Hypercore — the best sync model here, in the wrong language

Holepunch (Tether-backed) is a JS module stack: Hypercore (secure,
single-writer, Merkle-verified signed append-only logs with sparse
replication), Hyperbee/Hyperdrive (KV/files over logs), Autobase
(multi-writer merge with causal ordering), HyperDHT/Hyperswarm (Kademlia
discovery, UDP hole-punching, relay fallback), shipped in production in the
Keet app ([docs.pears.com](https://docs.pears.com/),
[Hypercore protocol](https://hypercore-protocol.github.io/new-website/protocol/)).
Identity is an ed25519 keypair per core — the log's public key _is_ its
identity — and an encrypted core's key functions as a read capability;
beyond that, auth is a `firewall` callback and app logic. The sync story is
the strongest in this survey (offline-first append, replicate on reconnect,
fetch only the ranges you want). But it is **JavaScript-only in practice**:
the sole Rust Hypercore is the unofficial, limited, pre-1.0 `datrs` port
with no Hyperswarm/Autobase equivalents
([hypercore on docs.rs](https://docs.rs/crate/hypercore/latest)). Its
transport competes with iroh and its runtime (Bare) is not Workers' JS
runtime either. Verdict: lose the stack, borrow the data model — signed
single-writer logs + Autobase-style multi-writer merge is exactly the sync
design to re-implement over iroh.

Scores: transport/NAT **4** · Rust maturity **1** · sync semantics **5** ·
project health **4** (funded and shipping; single-backer concentration) ·
iroh-estate fit **2**.

## 2.6 libp2p — the interop escape hatch, not the base

libp2p is the modular p2p networking stack/spec from the IPFS ecosystem:
the broadest transport menu available (TCP, QUIC-v1, WebSocket, WebRTC,
WebTransport; Noise or TLS 1.3; yamux), Kademlia DHT, gossipsub, and a
`PeerId` identity derived from the peer's public key
([libp2p.io](https://www.libp2p.io/docs/standalone-connectivity/)).
`rust-libp2p` is genuinely mature (facade 0.57.x) but pre-1.0 and
heavyweight, and NAT traversal is _assembled, not included_ — AutoNAT +
Circuit Relay v2 + DCUtR must each be wired and tuned
([libp2p on docs.rs](https://docs.rs/crate/libp2p/latest)). It has no sync
semantics and no capability/auth model beyond proving key possession. It
still wins where nothing else does: multi-language interop, existing
networks you must join (Ethereum consensus, Filecoin, IPFS), browser
transports, gossipsub at scale. The lineage matters for this estate: iroh
began as a Rust IPFS implementation and deliberately dropped libp2p —
multiaddrs, the dependency tree, assembled NAT — for key-addressed QUIC
endpoints; a `libp2p-iroh` bridge exists but is an early 0.1.x WIP
([iroh's own history](https://github.com/n0-computer/iroh.computer/blob/HEAD/src/app/blog/the-road-to-iroh-1-0/page.mdx),
[libp2p-iroh](https://docs.rs/crate/libp2p-iroh/latest)). Keep it as the
federation/interop hatch; do not stack it as the base — it duplicates
identity, discovery, and NAT machinery iroh already does more simply.

Scores: transport/NAT **3** · Rust maturity **4** · sync semantics **2** ·
project health **5** · iroh-estate fit **3**.

## 2.7 Freenet (2023+ rewrite) — whole-platform competitor; borrow two ideas

The new Freenet (ex-Locutus; the Java original became Hyphanet in 2023) is
a Rust platform for decentralized apps: a small-world ring where each
_contract_ (WASM, identified by code hash + parameters) has state
replicated near its ring location, kept current by subscription trees with
delta sync, under a custom UDP transport (X25519+AEAD, hole-punching,
public gateways for bootstrap)
([freenet.org FAQ](https://freenet.org/about/faq/),
[transport manual](https://freenet.org/build/manual/architecture/transport/)).
Its merge semantics are elegant — contract state must form an idempotent
commutative monoid, so updates apply in any order to the same result — and
its auth is platform-shaped: the contract's WASM _is_ the validity policy
(e.g. requiring a signature from a particular key), and _delegates_ are
local WASM agents that hold secrets and act via attributed messages. But
adopting it means abandoning iroh _and_ the local-first SQLite/Turso model
(state lives in the network, hosted by strangers' peers), under AGPL-3.0,
at 0.2.x alpha maturity with a ~440-peer live network. Borrow the monoid
merge-contract idea and the delegate pattern; adopt nothing.

Scores: transport/NAT **2** · Rust maturity **3** · sync semantics **4** ·
project health **3** · iroh-estate fit **1**.

## 2.8 ⚑ Holochain — composes with iroh today; the capability reference design

Holochain is an agent-centric framework (Rust conductor; app logic as WASM
"zomes", usually Rust): each agent keeps a signed source chain, and shared
data lives in a DHT whose holder-peers run the app's validation rules
before storing and serving entries — eventual consistency without global
consensus or merge semantics
([validation docs](https://developer.holochain.org/build/validation/)).
Two findings matter here. First, **current Holochain runs on iroh**: its
Kitsune2 networking layer has a transport "backed by iroh… using iroh's
QUIC-based networking", with a self-hosted bootstrap + relay server for
discovery and fallback — the only candidate in the survey that literally
composes with the estate's chosen transport instead of competing with it
([kitsune2_transport_iroh](https://docs.rs/crate/kitsune2_transport_iroh/latest),
[Holochain infra guide](https://developer.holochain.org/resources/howtos/running-network-infrastructure/)).
Second, it has **the only complete capability system in the p2p set**:
zome functions are inaccessible until an agent creates a `CapGrant`;
callers exercise it via `CapClaim` under Unrestricted / Transferable
(bearer secret) / Assigned (secret + authorized-key signature — "the
closest to a true capability") modes, revocable by deleting the grant, and
`genesis_self_check` validates a _membrane proof_ gating network admission
([capabilities docs](https://developer.holochain.org/build/capabilities/)).
The cost of adoption is architectural: the conductor + DNA/zome model, DHT
neighbourhood availability (an entry is retrievable only while its
authority peers are online), pre-1.0 churn after a decade of development,
and nothing that runs in Workers. Rational use: mine CapGrant/membrane/
validation as the reference design for the estate's own auth layer over
iroh; adopt the framework only for an app that genuinely wants
agent-centric validated-DHT semantics.

Scores: transport/NAT **4** (iroh's, inherited; docked for the bootstrap/
relay you must operate) · Rust maturity **4** · sync semantics **3** ·
project health **3** · iroh-estate fit **3**.
**⚑ Flag:** the only entry that _adds exactly what iroh lacks_ (validated
shared state + capabilities + admission membranes) _while reusing iroh_ —
deep-dive its capability design in synthesis even if the framework itself
is not adopted.

## 2.9 AT Protocol — borrow the key hierarchy, not the protocol

As an identity reference (not a transport): atproto's stable identifier is
a DID (`did:plc` or `did:web`) resolving to a document carrying the handle
(`alsoKnownAs`), the current signing key, and the current host (PDS)
endpoint — identity survives changing all three
([atproto DID spec](https://atproto.com/specs/did)). The borrowable core is
its **two-tier key split**: 1–5 priority-ordered _rotation keys_ control
the DID document and can replace _signing keys_ and repoint the host even
if the old host is offline or uncooperative (adversarial recovery), while
signing keys only sign the account's repository — a Merkle Search Tree of
records under signed commits, exportable whole as a CAR file for backup
and migration ([did:plc spec](https://github.com/did-method-plc/did-method-plc/blob/main/website/spec/v0.1/did-plc.md#how-it-works),
[repository spec](https://atproto.com/specs/repository)). For an estate
with its own CA/IdP this is a strictly better recovery pattern than "the
CA reissues everything", and the PLC operation log is worth copying as an
audit trail. Its capability story is thin (PDS-layer OAuth/service-auth
JWTs; a DID proves control, not scoped authority), official implementations
are TS/Go (Rust via the credible community Atrium SDK), and running actual
atproto is off-mission.

Scores: identity/key management **4** · capability/delegation **2** · Rust
maturity **3** · estate fit **3** · project health **5** (production at
Bluesky scale).

## 2.10 Noise + WireGuard — the channel layer; identity and nothing else

Noise is a framework for DH-based handshake protocols: long-term static
keys are the parties' identities, patterns (XX, IK, …) encode who proves
what when, and authentication is exactly "you hold the private key matching
the static key I expected" — naming, enrolment, and trust decisions are
explicitly the application's problem
([noiseprotocol.org](https://noiseprotocol.org/noise.html)). WireGuard is
one fixed Noise instantiation (`Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s`)
whose model is the purest form: a peer _is_ its 32-byte Curve25519 public
key, cryptokey routing binds keys to AllowedIPs, and there are no
certificates, enrolment, or revocation in-protocol — rotation and
revocation are config edits on every peer
([WireGuard paper](https://www.wireguard.com/papers/wireguard.pdf)).
Tailscale's architecture is the standing proof of how much control plane
(enrolment, key distribution, naming, ACLs, revocation) must be built
above that minimal core. Rust support is strong (`snow` for Noise;
Cloudflare's `boringtun` for userspace WireGuard). For this estate these
are the channel-security substrate _under_ other designs (iroh's QUIC/TLS
plays the same role in its stack) — they give a self-certifying peer
identity and forward secrecy, and pointedly nothing else: no capabilities,
no delegation, no groups, no lifecycle.

Scores: identity/key management **3** · capability/delegation **1** · Rust
maturity **4** · estate fit **4** · project health **4**.

## 2.11 SPIFFE/SPIRE — adopt the format and discipline, not the control plane

SPIFFE (CNCF-graduated standard) defines workload identity as a URI
(`spiffe://<trust-domain>/<path>`) carried in short-lived SVIDs — X.509
(URI SAN, for mTLS) or JWT (`sub`) — verified against a trust-domain
bundle; SPIRE, the Go reference implementation, adds the genuinely
distinctive piece: **two-stage attestation** (the server attests the node,
the on-node agent attests the workload from kernel/platform selectors) so
SVIDs are minted from observed evidence and auto-rotated (typically ~1h)
via the Workload API, with bundle-exchange federation between domains
([spiffe.io overview](https://spiffe.io/docs/latest/spiffe-about/overview/),
[CNCF graduation](https://www.cncf.io/announcements/2022/09/20/spiffe-and-spire-projects-graduate-from-cloud-native-computing-foundation-incubator/)).
The direct estate answer: a house CA issuing short-lived X.509 with a URI
SAN plus the existing OIDC provider issuing JWTs with the same URI as
`sub` _already is_ an SVID system in substance (SPIRE can even chain to an
existing PKI as upstream CA). SPIFFE adds only (1) attestation-based,
secretless issuance — real, and the house stack has no equivalent;
(2) standard wire formats for interop with Istio/Envoy and cloud
workload-identity federation; (3) naming/TTL discipline for free. Rust
support is client-only (the pre-1.0 `spiffe` crate: Workload API client,
SVID sources) — the runtime you'd operate is Go
([spiffe crate](https://docs.rs/crate/spiffe/latest)). Recommendation
shape: adopt the ID format + TTL/rotation policy now; deploy SPIRE only
when external interop or attestation becomes a concrete requirement.

Scores: identity/key management **5** · capability/delegation **2** (SVIDs
assert identity only) · Rust maturity **3** · estate fit **2** (full SPIRE;
format-only adoption scores far higher) · project health **5**.

## 2.12 Survey flags for synthesis (summary)

1. **Biscuit ⚑** — beats the field on _deployable offline token_:
   Rust-reference + official WASM + Datalog attenuation + public-key
   verification. The direct filler for capnp's "no offline authority"
   hole (§1.8).
2. **UCAN ⚑** — beats the field on _token semantics_ (delegation +
   invocation + policy + revocation; `did:key` = NodeId-shaped identity).
   Rust 1.0 is the blocker; treat as the semantic reference and a
   watch/borrow item.
3. **Holochain ⚑** — beats the p2p field on _composability + capability
   design_: runs on iroh today (Kitsune2), and CapGrant/CapClaim +
   membrane proofs are the reference design for an auth layer over iroh.
4. **AT Protocol patterns** — not a candidate, but its rotation/signing key
   split with adversarial recovery beats the estate's current
   CA-reissue-everything recovery story; borrow into the IdP/CA program.
5. **SPIFFE** — adopt ID format + short-TTL discipline without SPIRE;
   attestation is the one capability the house stack genuinely lacks.
6. **Holepunch** — sync _model_ (signed logs + Autobase) is the design to
   re-implement over iroh; the stack itself is JS-only and loses.
7. **libp2p** — keep as interop escape hatch only. **Veilid** and
   **Freenet** compete with the chosen transport and lose for this estate.
   **Macaroons** are prior art to cite. **Noise/WireGuard** are the channel
   layer, already subsumed by iroh's QUIC/TLS in the target design.

Cross-cutting: across all eleven survey entries plus capnp/capnweb, **no
single system provides identity + capabilities + sync + storage**; every
credible composition pairs a transport that proves key possession (iroh)
with a capability layer (live ocap à la capnp/Holochain, or offline tokens
à la Biscuit/UCAN) and a sync layer (log-based, à la p2panda/Hypercore
models). That is the shape synthesis should evaluate "p2panda for
everything" against.
