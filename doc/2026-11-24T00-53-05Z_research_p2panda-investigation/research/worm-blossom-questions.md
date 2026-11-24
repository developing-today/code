# Worm Blossom questions — Sneakerweb, vaporware audit, Reed, GPUI, ufotofu, Bab

Lane WB-QUESTIONS for the auth + transport investigation
(developing-today/code#13779). Researched 2026-11-23 from primary sources:
[worm-blossom.org](https://worm-blossom.org/) (the collective's weekly diary),
[sneakerweb.org](https://sneakerweb.org/), [bab-hash.org](https://bab-hash.org/),
the [Reed specification](https://worm-blossom.github.io/reed/),
[willowprotocol.org](https://willowprotocol.org/), docs.rs/crates.io, and the
[Riot Willow implementation audit](https://github.com/rabble/riot/blob/HEAD/docs/research/2026-07-10-willow-implementation-audit.md).
Baseline: the existing family note,
[willow-meadowcap.md](willow-meadowcap.md) — this document verifies, updates,
and extends it where Drew's questions go further. One baseline update matters
throughout: the Rust implementation has moved fast since the July 2026 audit —
`willow25` is now **0.7.11 (2026-11-06)** with 0.7.12-alpha.1 already out,
depending on `bab_rs ^0.8.1` and `ufotofu ^0.12.6`
([docs.rs: willow25](https://docs.rs/crate/willow25)).

---

## 1. What are SneakerWeb and Sneaker World?

### Sneakerweb — a web that travels on physical media

**Sneakerweb is a peer-to-peer protocol for publishing static websites with no
servers, no DNS, no registrars, and no hosting** — plus a released command-line
application implementing it. Its own one-liner: "a parallel web transported by
physical media" ([sneakerweb.org](https://sneakerweb.org/)).

- **The problem it addresses** is permission and infrastructure dependence in
  web publishing: to put a page on the normal web you need a domain registrar,
  DNS, and a host, all revocable and all rented. Sneakerweb removes all three.
  Websites live on users' devices and move between them on "the ultimate
  fallback infrastructure: physical storage media" — USB keys, SD cards, local
  networks — packaged as `.snk` files
  ([sneakerweb.org](https://sneakerweb.org/)).
- **Architecture** (from the [protocol specification](https://sneakerweb.org/spec)):
  sneakerweb is a thin application protocol over **Willow'25**. There is exactly
  one communal Willow namespace for the entire sneakerweb; each website is one
  Willow **subspace**, and the subspace's Ed25519 public key _is_ the site's
  "domain" (a one-to-one correspondence — claiming a domain means generating a
  keypair). Entry paths map directly onto URL paths, and a local web server
  serves your collection to your ordinary browser at
  `http://<base32-pubkey>.localhost:1312/<path>`. Willow is used, in their
  words, "to prevent the forgery of websites, merge updated data, and create
  efficient and compact .snk files" — `.snk` files are Willow **Drop Format**
  bundles. The protocol is deliberately transport-agnostic, but the spec
  _recommends_ `.snk` files exchanged via physical media or LANs only, and says
  so in strikingly normative terms: "these restrictions might seem cumbersome,
  but they will shape your interactions with this technology into a form that
  the internet cannot replicate"
  ([spec](https://sneakerweb.org/spec)).
- **Does it run?** Yes — it is the worm-blossom project's flagship _working_
  application. The CLI offers `domain`, `publish`, `export`, `import`, `block`,
  and `serve` commands, is downloadable from sneakerweb.org, and is iterated in
  public: a 1.2.0 announcement went out on the fediverse, and the most recent
  diary entries describe a performance release that replaced their homebrew
  server with axum, added seekable streaming video (150 MB files served through
  the `video` element), site-size indicators, and responsive layout
  ([worm-blossom.org diary](https://worm-blossom.org/)). They run in-person
  **sneakmeets** (DWeb Camp, V2_ Rotterdam) where people swap `.snk` files, and
  Sammy hand-delivered `.snk` files by train. A starter pack — "sammy's guide
  to the sneakerweb" as a downloadable `.snk` — exists for people with no sites
  to exchange yet.
- **Relation to Willow and to our estate**: sneakerweb is the proof-of-concept
  that the Willow data model + Meadowcap + Drop Format work end-to-end for a
  real (if deliberately small-scale) application. It is also a statement of
  values our estate does _not_ share: it is anti-scale and anti-internet by
  design. For the fleet, sneakerweb matters as evidence (the stack runs), not
  as a model (its transport stance is the opposite of ours).

### "Sneaker World"

There is **no separate product, protocol, or spec called "Sneaker World."**
In the diary, **"sneaker-world" is the collective's jocular name for the whole
scene around sneakerweb** — Sammy's weekly diary literally runs a recurring
beat headed "This week's news from sneaker-world:" to introduce sneakerweb
development news ([worm-blossom.org](https://worm-blossom.org/)). Read
generously, it names the emerging universe the protocol creates: the totality
of sneakerwebsites in circulation, the sneakmeets, the collectors swapping
`.snk` files — a "world" in the same sense as "the fediverse" names a scene,
not a piece of software. If Drew saw the term somewhere as if it were a
distinct thing, the diary usage is almost certainly the source.

---

## 2. Is Willow/Meadowcap vaporware?

Short answer: **no — but the family's centre of gravity is its specs, and the
layer a fleet would need most (network sync) is the layer that is still
paper.** Split by evidence:

### Shipping / works today

- **Two Final specs with independent implementations.** The Willow Data Model
  and Meadowcap are Final (Meadowcap since 21 Nov 2025), each implemented at
  least twice (Rust + TypeScript; a Go implementation exists as a third
  cross-check) — spec status per the baseline note, §0/§8
  ([willowprotocol.org](https://willowprotocol.org/)).
- **A real, fast-moving Rust implementation.** `willow25` on crates.io is at
  **0.7.11 (Nov 2026)**, with a dense release cadence (0.7.0 → 0.7.11 in about
  three months), a persistent fjall-backed store, and dependencies pinned to
  `bab_rs ^0.8.1` / `ufotofu ^0.12.6`
  ([docs.rs](https://docs.rs/crate/willow25)). The July 2026 Riot audit's
  pin recommendation (0.6.0-alpha.3) is already two minor lines behind.
- **A working application.** Sneakerweb (§1) is released, downloaded, used at
  events, and visibly iterated week to week — it exercises the data model,
  Meadowcap authorisation, the persistent store, and the Drop Format for real.
- **TypeScript implementations + Earthstar 11 (beta)** built on them
  (`willow-js` implements data model, Meadowcap, the older WGPS sync, and
  sideloading) — baseline note §8.

### Real but alpha / in-flight

- **The Rust store layer** is mid-construction in the open: recent diary
  entries describe Emmy implementing store _summarising_ and _splittable
  storage_ — the two ingredients of region-based set reconciliation — and
  wrestling the 3D-indexing problem with Amparore's AreaSlice work
  ([diary](https://worm-blossom.org/)). This is sync being built bottom-up,
  in public, not yet usable as sync.
- **Drop Format** is Candidate/Proposal-grade, shipping inside willow25 alphas
  with open payload-import issues (Riot audit, issue #51) — sneakerweb uses
  it, but the audit's advice was "do not use the alpha Drop Format" for
  evidence-grade work.
- **WTP (Willow Transfer Protocol)** — the current sync focus — is at the
  _message-encoding design_ stage in the diaries ("progress on defining
  message encodings for the WTP"), i.e. pre-implementation.

### Paper-only / paused

- **Confidential Sync** (the sync protocol with private interest overlap):
  Proposal status, and its own spec page says design is **paused**, to be
  redrafted after WTP (baseline note §0).
- **Private Interest Overlap**, **Willow URIs**, **LCMUX** (their stream
  multiplexing spec): proposals/designs without implementations.
- **Encrypted Willow** is explicitly _an analysis document, not a protocol_
  (baseline note §5.1).

### Credibility debits (the honest vaporware-adjacent evidence)

- **The bab_rs digest incident**: every `bab_rs` version before 0.7 computed
  _incorrect WILLIAM3 digests_; 0.8 "truly fixes" them (canonical changelog,
  quoted in the [Riot audit](https://github.com/rabble/riot/blob/HEAD/docs/research/2026-07-10-willow-implementation-audit.md)).
  A hash layer that silently produced wrong digests for multiple releases is
  the single hardest fact against the family's engineering maturity.
- **No hosted CI** in the canonical willow_rs repo (open issue #54 per the
  same audit).
- **Spec churn under the "Final" label**: a recent diary episode ("So, crisis
  within worm-blossom? Are we going to have to eat humble pie and change the
  specifications we've marked as FINAL?") records discovering that AreaSlices
  — needed by the store work — were not properly defined in the Final specs,
  and resolving it by re-homing the definition. The specs are living documents
  maintained by two-to-three people, not frozen standards.
- **Bus factor**: the collective is Aljoscha Meyer, Sam Gwilym, and (since 2026) Emmy Walker, grant-funded (NLnet), ~$3K/yr donations (baseline §12).

### Credibility credits

- Specs precise enough that independent implementations (willow-go) can serve
  as interoperability cross-checks — and caught digest discrepancies.
- External adopters do their own audits (Riot, §3) rather than trusting.
- The team publishes failures and design retractions in a weekly diary and in
  essays (e.g. "Timestamps, Really?") — vaporware projects do not usually
  document their own weakest points this carefully.

**Verdict**: Willow/Meadowcap is **not vaporware**: the data model, Meadowcap,
the Rust/TS implementations, and a real application demonstrably exist and
run. What _is_ still vapor, years in, is the confidential network sync layer
(Confidential Sync/PIO) and any production-grade Rust sync at all. An estate
adopting Willow today is adopting finished _data + authorisation_ layers and
volunteering to build or wait for the _network_ layer.

---

## 3. What other projects support Willow?

Ordered by seriousness of commitment:

- **Earthstar** — the origin and flagship. Willow began as a reimagining of
  Earthstar; Earthstar 11 is being rebuilt on Willow (`willow-js`,
  `@earthstar/meadowcap`), and Earthstar's core maintainer is one of the two
  Willow authors ([Comparison to Other Protocols](https://willowprotocol.org/more/willow_compared/)).
  This is first-party support, not external validation.
- **Sneakerweb** — worm-blossom's own application (§1). Also first-party, but
  it is the strongest existence proof that the full stack works end-to-end.
- **Leaf / Weird (muni.town)** — the most substantial _external_ design
  adoption. Leaf is a protocol family built on Willow that uses Meadowcap
  directly, and extends it where Willow is silent: **FROST threshold
  co-signing** so a capability can require an online co-signature — their
  answer to Meadowcap's missing revocation — plus petnames and contact-book
  identity for their app Weird
  ([Capabilities & Identity with Leaf](https://blog.muni.town/capabilities-and-identity-with-leaf/)).
  Nuance worth recording: the same team's later project (Roomy) evaluated
  Willow, then also explored Keyhive seriously because CRDT-native revision
  tracking fit chat better — evidence that even sympathetic adopters treat
  Willow as one ingredient, not the whole answer
  ([A Brief History of Roomy's Architectures](https://blog.muni.town/brief-history-of-roomy-architectures/)).
- **Riot (rabble)** — the deepest external _Rust_ adoption. Riot is a
  community application runtime being built directly on the `willow25`
  crates (Rust core + WASM browser node), and its team produced the July 2026
  implementation audit this note cites — pinning exact crate versions,
  working around the alpha Drop Format, vendoring willow25 for the browser
  ([audit](https://github.com/rabble/riot/blob/HEAD/docs/research/2026-07-10-willow-implementation-audit.md);
  [About Riot](https://riot-protest-net-marketing.protestnet.workers.dev/about/)).
  Riot is itself a prototype ("No stable release has shipped"), so this is a
  serious fellow-traveller, not a production reference deployment.
- **iroh-willow (n0)** — n0 (the iroh company) built a Willow + Meadowcap +
  sync implementation over iroh while exploring a successor direction to
  iroh-docs. It exists and its architecture is instructive, but it is
  unreleased and stale (iroh 0.34-era APIs, baseline note §10) — an
  experiment by a well-placed neighbour, not a supported product.
- **willow-go (Deln0r)** — an independent Go implementation used as an
  interoperability cross-check (it independently hit and fixed the WILLIAM3
  digest divergence). Its existence is evidence for spec quality (§2).
- **TagDrop (mofosyne)** — evaluated Willow as the import format for a
  dead-drop system and documented the fit assessment (including a real
  blocker: TagDrop signs with ML-DSA-44, Willow/Meadowcap authorises with
  Ed25519). An evaluation, not an adoption — included because it shows the
  shape of external interest: people _assess_ Willow often, _ship on it_
  rarely ([commit record](https://github.com/mofosyne/tagdrop/commit/ce0e3105c895aa155c5de54382a7edaf396a64b3)).
- **Research uptake** — Amparore's storage-indexing analysis (2026) treats
  Willow's 3D structure as a research object in its own right (baseline §6).

Summary: support is real but thin and mostly first-party or prototype-grade.
Nobody is running Willow in production infrastructure; the two external
projects that matter (Leaf, Riot) are pre-release and are both, tellingly,
building _around_ Willow's gaps (revocation, sync) rather than waiting for
worm-blossom to close them.

---

## 4. Is Willow the most "pure/correct" option — fewest shortcuts, most theoretically right?

**Judgment: within its variety — replicated data stores with capability
authorisation — yes, Willow is the most formally rigorous and least
shortcut-taking design currently under active development. Its purity is
real, and it is also precisely where its practical costs come from.**

Where the formalism genuinely leads the field:

- **Spec precision as the product.** The data model is defined as a join
  semi-lattice with exact join/prune rules; Meadowcap validity is a recursive
  definition tight enough to implement from the paper alone — and two
  independent codebases did. Compare p2panda (engineering-led, spec follows
  code) or Nostr (deliberately laissez-faire — the Willow authors' own
  comparison concedes Nostr is "significantly easier to implement, but
  arguably also more difficult to _solidly_ build upon"
  ([Comparison](https://willowprotocol.org/more/willow_compared/))).
- **The Area algebra.** Subspace × path-prefix × time-range areas, closed
  under intersection, serve simultaneously as the unit of authorisation
  (Meadowcap), the unit of interest (partial sync), and the unit of
  reconciliation (AreaSlices in RBSR). One concept, three jobs, no seams —
  nobody else in the survey has an authorisation scope that is _the same
  object_ as their sync scope.
- **Attenuation as a theorem, not a policy.** Meadowcap delegation validity
  _requires_ area inclusion at every hop; widening is cryptographically
  unverifiable (baseline §2.3). Keyhive's Keyline matches this rigour in the
  auth layer specifically (set-based evaluation, RT₀-style reasoning) — it is
  the one peer that plays the same purity game, on a narrower board.
- **Private Interest Overlap.** Willow is the only family that even attempts
  metadata-private sync discovery — peers learn only their shared interests.
  The authors' comparison notes, fairly, that "few if any protocols devote
  the attention to controlling data propagation through read capabilities,
  and even fewer extend this care to metadata privacy"
  ([Comparison](https://willowprotocol.org/more/willow_compared/)).
- **Refusing the easy outs.** No content-addressing at the protocol level
  (they argue it, at length, against IPFS-style designs); no server/ACL
  fallback for revocation — instead a published essay dissecting why
  timestamps are a compromise ("Timestamps, Really?"); no completeness
  proofs, _deliberately_, to keep traceless deletion possible. When their
  hash needs (constant-size length proofs) weren't met by Bao, they designed
  a hash family (§8) rather than accepting the logarithmic proof. When their
  log scheme (Bamboo) was beaten by their own newer result, they published
  Reed (§5) — and then abandoned logs anyway when the data model changed.
  This is a research programme behaving like one.

Where purity costs — the same choices, read from the operator's side:

- **No revocation** is a _theorem-shaped_ hole: offline-verifiable
  capabilities cannot be revoked without an online element, so Meadowcap
  doesn't pretend — and every adopter (Leaf's FROST co-signer, our renewal
  service idea) ends up bolting the online element back on.
- **Claimed timestamps** are load-bearing for both conflict resolution and
  capability windows; the design trusts writers with time because a purer
  alternative (causal history) would break the state-based model. The purity
  is in _admitting_ this, not in solving it.
- **Storage efficiency is an open research problem** (no lexicographic index
  makes every Area contiguous — Amparore 2026): the 3D formalism is
  beautiful and, as yet, nobody knows how to store it optimally.
- **The network layer is unfinished after years** precisely because the
  confidential-sync design refuses the straightforward version (sync first,
  privacy later). Purity has a schedule cost, and Willow pays it in the open.

Against the rest of our survey: p2panda is the pragmatic engineer (compose
iroh, ship crates, let auth be simple); the ocap family is _older_ theory
(decades of capability literature) but solves a different problem (live
objects, not replicated data); Keyhive is Willow's only true peer in rigour,
and it is even less mature. So: **Willow is the leading-edge research
artefact of its class — the one you'd cite, teach, and steal designs from.
That is a different claim from "the one to bet a fleet on today," and the
family's own behaviour (specs Final, sync paused, implementations alpha)
keeps those two claims separate.**

---

## 5. What is Reed's prefix auth scheme? Is it good?

**First, a naming correction that changes the answer: "prefix auth" is
prefix _authentication_, not prefix _authorisation_.** Reed has nothing to
do with Meadowcap-style path-prefix capabilities. It belongs to the other
meaning of "auth" entirely.

### What it is

**Reed** is Aljoscha Meyer's lightweight specification of a **prefix
authentication scheme** — the cryptographic primitive behind append-only /
transparency logs ([Reed Specification](https://worm-blossom.github.io/reed/)).
Given a single totally-ordered stream of events, a prefix authentication
scheme lets anyone produce a _short certificate_ proving that one event
happened before another — equivalently, that the sequence of events up to
position _i_ is a prefix of the sequence up to position _j_. This is the
primitive Certificate Transparency uses (signed tree heads + inclusion /
consistency proofs), generalised.

The construction, from the spec:

- Build a Merkle-DAG over the event sequence: events get **commitment
  vertices**, plus inner vertices arranged as a **ternary almost-skip-list**
  (inner vertices sit at positions (i, j) where 3^j divides i — hence short
  paths between any two positions).
- Each vertex is labelled by a hash of its out-neighbours' labels (plus
  position metadata, which lets any label prove its own position). Each
  commitment vertex can reach the commitment vertices of _all earlier_
  events.
- The **prefix certificate** for two events is the set of labels of the
  closed out-neighbourhood of the shortest path between their commitment
  vertices; from those labels a verifier recomputes both commitment labels,
  and matching labels unforgeably certify the ordering. Path-finding is a
  simple greedy walk (jump if you don't overshoot, else step to predecessor).
- For a transparency-log deployment, a logging authority signs commitment
  vertices, which then play the role of CT's signed tree heads.

Reed is the spec form of the scheme from Meyer's paper **"Better Prefix
Authentication"** ([arXiv:2308.15058](https://arxiv.org/abs/2308.15058)),
which recasts these schemes as _antimonotone linking schemes_ and improves
on the state of the art in both prefix authentication and secure relative
timestamping — explicitly positioned as "more efficient alternatives to
certificate transparency logs." Reed **supersedes Bamboo**, Meyer's earlier
log format (the one old p2panda was built on): the Reed spec says it is "more
efficient than Bamboo, more minimalistic in feature set, and generic over any
particular cryptographic primitives," and the Bamboo repo now points readers
to Reed ([Bamboo](https://github.com/AljoschaMeyer/bamboo)). Tellingly, the
Reed spec also records _why_ it shed features: Bamboo was designed for data
replication, "but I have since come to prefer more flexible replication
technologies" — i.e. Willow.

### Is it good?

- **As a design: yes.** It comes with a paper, a complexity analysis, and a
  cleaner construction than its predecessor (shorter proofs than CT logs;
  fewer moving parts than Bamboo's lipmaalink certificate pools). It is the
  same author's second, better attempt at a problem he had already shipped
  once — usually a good sign.
- **As an authorisation scheme: it isn't one.** The comparison the question
  invites — Reed vs Meadowcap areas vs Keyline — is a category distinction,
  not a contest. Meadowcap and Keyline answer "who may do what to which
  data"; Reed answers "can you prove event A precedes event B in this one
  stream". Reed could sit _under_ a log-based system (it is what Bamboo was
  for p2panda); it cannot substitute for a capability layer, and a capability
  layer cannot substitute for it.
- **As an adoption target today: weak.** No production implementation
  surfaced in crates.io/Codeberg/GitHub searches — the spec site is the
  artefact, and unlike Bamboo it has no flagship user. Its author has moved
  on twice over: from logs to Willow's set-based model, and in a companion
  paper (arXiv:2308.13836) he publicly retracts the "append-only log"
  framing itself. Reed is best read as a finished piece of research whose
  natural habitat (single-writer log replication) its own author no longer
  inhabits.
- **For our estate**: Reed becomes relevant only if we build a component
  that genuinely needs prefix proofs — a transparency log for the fleet CA,
  or an audit log whose consumers must verify ordering without trusting the
  broker. There, it is a better primitive than a hand-rolled Merkle log and
  worth implementing from the spec. For access control, it is not a
  candidate at all.

---

## 6. What is their opinion on GPUI?

The honest framing first: **worm-blossom has no GPUI essay and does not use
GPUI.** Their UI surface is the ordinary web — sneakerweb serves HTML from a
local axum server to your normal browser. What exists is a diary link-roundup
gloss, and it is unambiguous about which side they're on.

In a recent weekly diary, Sammy's link list includes
[**A Critique of GPUI**](https://blog.zortax.de) (by the author of the
signal-based `zgui` framework) with this worm-blossom gloss, verbatim in
substance: _"A nuanced take on where Rust UI libraries inspired by Elm
(which feels like all of them really) fall flat. Personally I loved Elm but
found the lack of component-level state very fiddly. And there's lots to
suggest the lack of a component model makes it much harder to calculate diffs
between renders"_ ([worm-blossom.org diary](https://worm-blossom.org/)).

So their position, as far as it is stated:

1. **Endorsement of the critique.** The linked post argues GPUI's (and
   egui's) state models are dead ends for complex apps: GPUI's `notify`
   names an entity but not what changed, so invalidation re-renders whole
   view subtrees — cost follows tree size, not change size (their benchmark:
   ~8.4 ms for a one-label change at 3,000 live rows) — and fixing it means
   hand-building a signal graph out of heavier parts. The field's converged
   answer, per the critique, is tracked reads / signals with run-once
   components ([DevPlace mirror](https://devplace.net/news/26192b3ad0cb-a-critique-of-gpui)).
2. **The specific worm-blossom complaint is architectural, not about
   rendering speed**: Elm-style architectures (a single global model +
   message updates, which they see GPUI as belonging to) lack
   _component-level state_, which makes both programming (fiddly) and
   efficient diffing (no component boundaries to diff against) harder.
   Note the affection in it — "Personally I loved Elm" — this is a
   disappointed-former-believer critique, not a dismissal of the paradigm's
   goals.

Caveats: this is one link blurb, not a developed position; no other GPUI
discussion appears on the site (searches of the diary and archives return
only this entry); and they have no skin in the game — no worm-blossom
project ships a native GUI. Weight it as "informed bystanders who agree with
the signals critique," not as a framework evaluation.

---

## 7. What is ufotofu?

**Ufotofu is worm-blossom's Rust library of abstractions for asynchronously
working with series of data — streams and sinks, redesigned from first
principles** ([worm-blossom.org/ufotofu](https://worm-blossom.org/ufotofu/);
[docs.rs: ufotofu 0.12.7](https://docs.rs/crate/ufotofu/latest/source/src/lib.rs)).
It is the async idiom the whole Willow/Bab stack is written in, and Emmy
Walker joined the collective largely on the strength of ufotofu contributions
([diary](https://worm-blossom.org/)).

How it works:

- **Two dual traits.** A _sequence_ is items terminated by at most one final
  value. A `Producer` yields `Either<Item, Final>` one at a time (with
  `slurp` to fill internal buffers); a `Consumer` receives them (with
  `flush` to empty buffers). Both can error on any call. The traits are
  designed as exact duals, which makes composition (piping a producer into a
  consumer; in-memory channels whose sender/receiver are the two halves)
  fall out naturally.
- **Bulk traits for zero-copy.** `BulkProducer`/`BulkConsumer` expose slices
  — a closure writes into a consumer-exposed mutable slice, or reads from a
  producer-exposed one — so byte-oriented code avoids per-item overhead
  while remaining semantically identical to the item-wise traits.
- **Codec modules.** `codec` provides async encoding/decoding traits on top
  of producers/consumers; `codec_relative` does the same where encodings may
  exploit shared context between encoder and decoder (exactly what Willow's
  relative encodings for capabilities need).
- **Frugal async.** Ufotofu follows the team's "frugal async Rust"
  principles: futures from its methods are **`!Send`** (single-threaded
  executors only), `no_std`-compatible, with explicit buffering instead of
  hidden allocation. Compatibility adapters turn any futures-io
  `AsyncRead`/`AsyncWrite` into bulk producers/consumers, so Tokio/smol I/O
  can be bridged in at the edges.

Maturity and use: actively developed (0.12.7; Codeberg PRs through 2026;
combinator ecosystem — Limit/Chain/etc. — with fuzz-testing utilities);
used by `willow25` (depends on ufotofu ^0.12.6,
[docs.rs](https://docs.rs/crate/willow25)), by sneakerweb (Emmy built
ufotofu↔axum streaming adapters to serve seekable video,
[diary](https://worm-blossom.org/)), and it has a small devoted following
(the diary's guest poster describes "morphing into the ufotofu sicko I was
perhaps always meant to be").

**Relevance to our Rust work**: ufotofu itself is a respectable, principled
alternative to futures-rs `Stream`/`Sink` for streaming _serialisation_
workloads — the Final-value-in-the-type design genuinely fits protocol
codecs (a stream that ends with a checksum/status is one type, not a
convention). But adopting the Willow crates means adopting ufotofu's async
worldview with them: `!Send` futures and pollster-style executors sit
awkwardly inside a Tokio multi-threaded fleet binary, and bridging is
per-boundary work. As a standalone choice for _our own_ new streaming code
it is a small-ecosystem bet (one collective, no Tokio-native ecosystem) —
interesting where its codec discipline pays, not a default.

---

## 8. Should Drew use `bab`? Why bab and not BLAKE3?

### What bab is

**Bab is a family of hash functions designed for verifiable partial data in
peer-to-peer systems** — spec at [bab-hash.org](https://bab-hash.org/spec),
Rust implementation `bab_rs`. Like BLAKE3/Bao, a Bab digest is the root of a
binary Merkle tree over 1024-byte chunks, so a receiver can verify a stream
_as it arrives_ (each verified prefix is safe to persist and forward),
verify arbitrary _slices_ independently, and pull different slices from
different peers in parallel. **WILLIAM3** is the family's concrete
instantiation — the one Willow'25 uses as its payload hash — and it is,
deliberately, _almost_ BLAKE3.

### Why not just BLAKE3 (+ Bao)? — their reasoning, in their words

Meyer's [Bab Introduction](https://aljoscha-meyer.de/posts/bab_funding/index.html)
gives three advantages of Bab over Bao (the verified-streaming spec layered
on BLAKE3):

1. **Constant-size length proofs.** Given a Bab digest, a peer can certify
   _how many bytes_ were hashed in constant space. Bao needs logarithmic
   space for the same proof — small, but growing, and it complicates
   storage-oriented APIs.
2. **A generic, parameterisable construction.** Bab is specified over
   abstract `hash_chunk`/`hash_inner` functions (there is an instantiation
   over any conventional hash function); Bao is tied to BLAKE3's specific
   primitives. Bab is "a family of special-purpose hash functions" where Bao
   is "a byproduct of a well-designed general-purpose hash function"
   ([spec §1](https://bab-hash.org/spec)).
3. **Streaming optimisations Bao doesn't have** — omitting left labels,
   omitting lower layers, slice streaming, and optimised computation for
   repeating substrings ([spec §4](https://bab-hash.org/spec); the
   [bab-hash.org](https://bab-hash.org/) homepage lists the same deltas as
   "APIs designed from a storage rather than a hashing perspective").

WILLIAM3's three concrete differences from BLAKE3 implement exactly this
([spec §2.2.2](https://bab-hash.org/spec)): (i) **no chunk indices** in chunk
labels (BLAKE3 feeds the chunk counter into its compression function;
WILLIAM3 sets it to 0) — this is what makes chunk labels position-independent
and the repeating-substring optimisation possible; (ii) **the substring
length is mixed into inner-node labels** (the compression function's `t`
parameter carries the length) — this is what makes constant-size length
proofs work; (iii) **different IV constants** (the BLAKE3 digest of the
ASCII string `WILLIAM3`) so WILLIAM3 and BLAKE3 digests can never be
confused for the same input. WILLIAM3 drops BLAKE3's KDF mode and XOF
output; width 32 bytes, chunk size 1024, keyed mode retained.

And why Willow cares: a Willow payload digest is not just a checksum — it is
the verifiable-stream root for resumable payload transfer and an input to
set-reconciliation fingerprints. Plain BLAKE3 gives Willow none of the
streaming semantics; Bao gives most of them with worse length proofs and a
second spec to track. So Willow'25 standardised on WILLIAM3.

### Correctness state — read this before pinning anything

- **Every `bab_rs` before 0.7 computed incorrect WILLIAM3 digests.** The
  canonical changelog (quoted in the
  [Riot audit](https://github.com/rabble/riot/blob/HEAD/docs/research/2026-07-10-willow-implementation-audit.md))
  says 0.8 "truly fixes" them. `willow25` 0.6.0-alpha.1 regenerated its
  default digests when it moved to bab_rs 0.8; current `willow25` 0.7.11
  depends on `bab_rs ^0.8.1` ([docs.rs](https://docs.rs/crate/willow25)).
- The independent Go implementation also had to correct its WILLIAM3
  (commit 9d848ee, per the audit) — the divergence was in the wild, on both
  sides.
- Practical meaning: **WILLIAM3 digests produced before bab_rs 0.8 are a
  different hash function in effect.** Any stored digests from the 0.5/0.6
  era are not comparable to current ones. Pin `bab_rs = "=0.8.1"` (or the
  exact version your willow25 resolves), keep the lockfile as the evidence
  of which digest-world you are in, and expect that a young spec like this
  may do it once more before 1.0.

### Recommendation for this estate

- **Use bab where it comes with the territory**: if we adopt Willow'25 data
  (or build Willow-interoperable tooling), WILLIAM3 via `bab_rs =0.8.1` is
  not optional — payload digests and reconciliation fingerprints are defined
  over it. Budget for digest-world versioning (record the bab_rs version
  alongside any persisted digest).
- **Do not adopt bab as the estate's general-purpose hash.** BLAKE3 is
  faster (SIMD-tuned, massively deployed), and the estate already lives in
  the BLAKE3 world via iroh-blobs/bao. Bab's unique wins — constant-size
  length proofs, storage-shaped streaming APIs — only pay inside a
  Willow-style store/sync design; outside one, they buy nothing and cost a
  second, single-implementation, pre-1.0 hash ecosystem with a demonstrated
  silent-digest-bug history.
- **What using BLAKE3 instead would break**: Willow'25 _interoperability_ —
  a BLAKE3-digested store cannot reconcile with willow25 peers (digests and
  fingerprints won't match), and you lose constant-size length proofs. What
  it would _not_ break: local correctness. iroh-willow already proved a
  BLAKE3-instantiated Willow works fine as a closed system (baseline note
  §10). So the decision is exactly: **interop with the Willow'25 world →
  bab/WILLIAM3; closed system on our own transport → BLAKE3/bao is the
  safer default.** Given the estate's iroh direction, bab adoption should
  follow a Willow adoption decision, never precede it.

---

## Sources (primary)

- worm-blossom.org — [homepage/diary](https://worm-blossom.org/),
  [projects page](https://worm-blossom.org/projects/),
  [ufotofu](https://worm-blossom.org/ufotofu/)
- [sneakerweb.org](https://sneakerweb.org/) and its
  [protocol specification](https://sneakerweb.org/spec)
- [bab-hash.org](https://bab-hash.org/) and the
  [Bab specification](https://bab-hash.org/spec); Meyer,
  [Bab Introduction](https://aljoscha-meyer.de/posts/bab_funding/index.html)
- [Reed Specification](https://worm-blossom.github.io/reed/); Meyer,
  [Better Prefix Authentication](https://arxiv.org/abs/2308.15058);
  [Bamboo](https://github.com/AljoschaMeyer/bamboo)
- willowprotocol.org —
  [Comparison to Other Protocols](https://willowprotocol.org/more/willow_compared/),
  [Meadowcap](https://willowprotocol.org/specs/meadowcap/index.html)
- docs.rs — [willow25](https://docs.rs/crate/willow25),
  [bab_rs](https://docs.rs/bab_rs/0.8.1/src/bab_rs/lib.rs.html),
  [ufotofu](https://docs.rs/crate/ufotofu/latest/source/src/lib.rs)
- Rabble, [Riot Willow Implementation Audit](https://github.com/rabble/riot/blob/HEAD/docs/research/2026-07-10-willow-implementation-audit.md);
  muni.town, [Capabilities & Identity with Leaf](https://blog.muni.town/capabilities-and-identity-with-leaf/),
  [A Brief History of Roomy's Architectures](https://blog.muni.town/brief-history-of-roomy-architectures/)
- zortax, [A Critique of GPUI](https://devplace.net/news/26192b3ad0cb-a-critique-of-gpui)
  (linked and glossed by worm-blossom)
