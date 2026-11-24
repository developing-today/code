# Content addressing for the ID system: Bao, Bab, and the item layer

**Kind:** research / decision input · **Date:** 2026-11-23 · **Lane:** CONTENT-ADDRESSING-RESEARCH
**Consumers:** D2 — transclusion and embedding of ID objects (#13887, this is its
hash-layer input); ID-graph epic #13880; P2P synthesis #13779 (its decision
point 5, the Willow data-model question, is partly this question); transport
adapter contract #13929.
**Seed hypothesis under test:** Archer's chat comparison of 2026-11-23 —
Bab is structurally advantaged over Bao for large nested ID blobs that
normalize into cross-referenced, transcluded, partly versioned items,
because Bab's chunk labels zero out position (identical substrings hash
identically anywhere) while Bao's identity is positional (root + offset +
length); with Bao-per-item + manifest as the middle path. This research
tests that hypothesis against the specs, the implementations, and
measurements. **Verdict in advance: the hypothesis is right about identity
and proofs, wrong if read as a claim about version updates — and the
"middle path" turns out to be the main road.**

---

## 0. Recommendation (decision-grade)

1. **Decide the item layer first; it carries most of the value under every
   hash choice.** Address normalized items individually — each item is a
   payload named by (digest, length) — and address collections as
   manifests: ordered lists of item digests (the HashSeq shape). Mutable
   names stay one layer up as signed name→digest pointers. This is also
   what every already-decided component assumes: p2panda operation
   headers carry a BLAKE3 payload hash + payload size; Willow entries
   carry payload_digest + payload_length; iroh-blobs collections are hash
   sequences; D2's snapshot embed is defined as digest-bound. Measured
   effect on the representative corpus (§5): an edit-and-republish cycle
   costs **1.3%** of the collection under per-item addressing, vs **89%**
   (Bab) and **100%** (Bao) for whole-blob fixed chunking, and **16%** for
   content-defined chunking.
2. **For the payload/tree layer today: the Bao family (BLAKE3), via
   iroh-blobs / bao-tree, through p2panda-blobs.** It is the decided data
   plane's own payload layer (p2panda-blobs wraps iroh-blobs), it is the
   most production-hardened option in the field, its wire protocol exists
   and is deployed (ALPN `/iroh-bytes/4`), and at item scale (payloads of
   a few KiB) Bao's real weaknesses — positional identity, no cheap
   length proof — barely bind. For bulk blobs between our own servers,
   positional verified streaming is exactly the right tool, and it
   composes with the Dragonfly verdict on #13929 (digest pinned over our
   signed channel; the carrier itself need not be trusted).
3. **Designate Bab/WILLIAM3 the tracked successor for the payload digest,
   gated on maturity triggers (§7) — do not adopt it yet.** Its
   advantages are real and spec-verified (§3.3): position-free labels
   make identical subtrees the same object in every blob; length-
   committed labels give constant-size (72-byte) length proofs and
   self-locating slices. Those advantages bind at collection scale and
   on tiny carriers. But bab_rs computed *incorrect WILLIAM3 digests in
   every version before 0.7* (the current pin, 0.8.1, "truly fixes"
   them), fuzzing found integer overflows in it during 2026, there is no
   finished network wire protocol for Bab payloads (WTP is a
   pre-implementation draft), and the upstream is a two-person,
   grant-funded team. Adopting Bab today means forking the payload layer
   away from iroh-blobs' gravity — the iroh-willow precedent shows the
   cost: it instantiated Willow with BLAKE3 digests and thereby was *a*
   Willow but not Willow'25; the hash-world choice is an interop choice.
4. **Make the future flip cheap now: algorithm-tagged digests from day
   one.** D1 URIs and D3 objects should carry digests in a tagged form
   (multihash-style: algorithm + digest + length), never a bare 32-byte
   hash with the algorithm implied. Then a later BLAKE3→WILLIAM3 flip is
   a re-hash of payloads behind an unchanged interface — the graph, the
   manifests, and the name layer do not move. This is the single
   cheapest insurance in this whole research.
5. **CDC chunking stays out of the peer protocol.** It measured well on
   updates (16%) but its chunks cannot be named, sliced, or verified
   standalone in a D1 URI, and per-item addressing beats it on
   item-shaped data anyway. It remains legitimate *inside* one
   administrative domain (backup/archival tiers, restic-style) where the
   manifest arrives over our own signed channel.
6. **Manifests: flat now, tree'd when they grow.** A 403-entry manifest
   is 16 KB — a flat HashSeq is fine. If a collection's manifest passes a
   few hundred KiB, give it the prolly-tree / Merkle-search-tree shape
   (structural sharing between manifest versions, diff in time
   proportional to the change) rather than inventing a format. That is a
   D3 manifest-format decision, not a hash decision.

**What to prototype next (named, not enqueued):** the D2 snapshot
fixture (post-embeds-photo-collection, #13887 slice 1) built twice behind
one (digest, length) interface — once on iroh-blobs via p2panda-blobs,
once on a bab_rs-backed store — plus a digest-agility test-vector set for
D1 (the same items addressed under BLAKE3 and WILLIAM3, URI forms side by
side). That prototype is the evidence base for the flip decision when the
§7 triggers fire.

---

## 1. The question, decomposed

"Which content addressing?" is three questions that are easy to conflate
and expensive to confuse:

- **The tree/hash layer.** Given a byte string, what is its digest, what
  does a proof for a slice of it look like, and when are two byte ranges
  "the same piece"? (Bao, Bab, flat hashing.)
- **The item layer.** How is a large logical object decomposed into
  addressable units, and how are the units named as a set? (Per-item
  digests + manifests, HashSeq collections, Merkle-DAGs, prolly trees.)
- **The sync layer.** Given two replicas at different versions, what
  crosses the wire? (Digest-set resync over pieces; range requests
  against a named root; set reconciliation over entry sets.)

The seed comparison was framed at the tree layer (Bab vs Bao). The
measurements in §5 show the item layer dominates outcomes for our
shapes, and the tree layer decides the remainder: proof semantics,
cross-item identity, and carrier fit. Any recommendation that picks a
hash without fixing the item layer first is answering the smallest of
the three questions.

**Our shapes** (from the ID-graph programme, #13880 and children):

- Payloads **normalize into items**: a large nested blob becomes many
  individual objects that cross-reference each other (D3 graph layer).
- **Transclusion** is a first-class verb (D2, #13887): live-reference
  embeds and digest-bound **snapshot** embeds; collection embeds (a
  whole subgraph in a page) are the bulk case. D2 slice 1 must decide
  what a snapshot freezes — payload digest is the candidate this
  research feeds.
- **Versioned replacement**: an item is superseded; a signed pointer one
  layer up names the current digest. Neither hash layer provides the
  mutable name; the hash layer decides the *cost* of a new version.
- **Digest-pinned references** (D1, #13886): the URI scheme names a
  single object "with optional payload slice and optional expected
  digest" — slice references are in the *naming* layer, so slice-proof
  efficiency is a naming property, not just a transfer property. D1 also
  requires offline verifiability on weak carriers (paper, email headers,
  QR, sneakernet bundles).
- **Multi-transport reach** (#13926/#13929): digest references must
  survive low-bandwidth carriers — the extreme case in the wider
  ecosystem is a 237-byte Meshtastic packet (Riot's LoRa profile keeps
  every message inside one 1024-byte WILLIAM3 chunk so it stays a single
  verification unit). Proof size is a hard constraint there, not an
  optimization.

---

## 2. What the decided stack already assumes

The estate is not choosing on a blank field. The committed layers lean
specific ways, and this research weighs that gravity honestly:

- **p2panda is the data plane** (composition A, #13779). Its operation
  headers already commit to a payload as **BLAKE3 hash + payload size**.
  Its blob story is a decided wrapper: **p2panda-blobs wraps iroh-blobs**
  (BLAKE3/Bao verified streaming) "for large files that don't belong in
  operation bodies" (p2panda research note, crates.io record). Choosing
  Bab for payloads means diverging from the data plane's own payload
  layer on day one.
- **Willow is a data-model candidate** (decision point 5), and Willow'25
  instantiates payload hashing as **WILLIAM3** with entries carrying
  payload_digest + payload_length. If any domain earns the Willow model,
  its payloads live in the Bab hash world; the estate would then run two
  hash worlds or flip one side — iroh-willow's history (§3.4) is the
  worked example of that fork.
- **iroh is the transport**, and iroh-blobs (0.103.0, the family's most
  production-hardened protocol) is BLAKE3/bao-tree by construction. The
  estate runs iroh 1.5.0. Blob interop with the wider iroh ecosystem
  (sendme and friends) is Bao-shaped, full stop.
- **Meadowcap** is the capability layer (as spec, composition A). It is
  comparatively hash-agnostic — its own instantiation notes record
  P-384 as spec-legal for Meadowcap — so the capability layer does not
  decide this question; the token layer's curve settlement (Ed25519 /
  P-256 under composition C) is orthogonal to the payload hash.
- **Dragonfly** (#13929 assessment): not a content-addressing system at
  all — its digests prove consistency inside one trusting cluster, not
  publisher authenticity. It composes *under* any of these choices when
  `Download.digest` is pinned from our signed channel: the digest is the
  commitment, Dragonfly is a mover. It neither needs nor rewards Bab.

Net: the decided stack assumes (digest, length) payload references
everywhere, BLAKE3/Bao at the blob layer today, and leaves exactly one
door open — the digest *algorithm* is nowhere constitutionally fixed,
which is why recommendation 4 (tagged digests) is available at all.

---

## 3. The contenders

### 3.1 Bao over BLAKE3 (vanilla)

**Identity.** A blob's digest is its plain BLAKE3 hash (the Bao spec is
explicit: "the `bao hash` of a file is the same as the regular BLAKE3
hash"). Chunks are 1024 bytes; chunk chaining values are computed with
the **chunk counter** as an input — the same bytes at a different offset
have a different chaining value. Identity is therefore positional in the
strong sense: a reference is (root, offset, length), and no sub-part of
a blob has an identity of its own.

**Proofs.** A slice carries an 8-byte length header plus the parent
nodes on the traversal to the requested chunks (64 bytes per parent
pair). Exact sizes, computed from the tree geometry (§5): a one-chunk
slice proof is 648 B at 1 MiB, 1,096 B at 100 MiB, 1,288 B at 1 GiB.
Verification is one BLAKE3 compression per chunk in the slice plus one
per proof node — the cheapest verification in the field, on the most
optimized hash implementation in the field.

**Length.** Bao's wart, and it is a design wart not an implementation
one: the length header is *not* committed by the root hash. The spec's
"final chunk requirement" states the length is validated **when and
only when the decoder validates the final chunk**, and decoders must
not expose length before that. Proving a blob's total length therefore
costs a right-edge traversal plus the final chunk (~1.7 KB at 1 MiB,
~2.3 KB at 1 GiB) and a decoder-discipline rule every implementer must
get right.

**Sharing/versioning.** None below the whole blob. Identical substrings
are not recognized; an edit anywhere produces a new root under which no
piece of the old version is addressable. (A client holding both
versions' *bytes* can range-diff locally, rsync-style — but it cannot
fetch the unchanged ranges from a third party *by name*, nor verify
them as pieces of the new root without their proofs. Under digest-set
resync — the content-addressed sync model — reuse across versions is
zero. §5 measures exactly this.)

**Storage.** The outboard (all parent nodes) is 64 B × (chunks − 1) +
8 ≈ **6.25%** of content. This is vanilla Bao's real cost and the reason
n0 rewrote it (§3.2).

**Maturity.** Spec is short, stable, and multiply implemented; BLAKE3 is
audited, ubiquitous, and fast (SIMD, multi-GB/s class). Bao itself is a
byproduct of the BLAKE3 project — boring in the best way.

### 3.2 bao-tree / iroh-blobs (Bao, production profile)

n0's re-implementation keeps BLAKE3 digests (interop with Bao's root
hashes) but stores the outboard only down to **16 KiB chunk groups**
(BlockSize = 2^4 chunks): the tree's leaves are group chaining values,
computed over the group's 16 chunks internally.

- Outboard drops to ≈ **0.39%** of content (4,040 B at 1 MiB; ~4.2 MB at
  1 GiB).
- The price is **verification granularity**: a requester detects bad
  data "after at most 16 KiB", and the minimum useful transfer for a
  small range is a whole group. Measured (§5): fetching a 1.8 KiB item
  out of a 1 MiB collection costs 16,776 B; fetching 12 small embedded
  items costs **236 KB for 28 KB of content** — group rounding dominates
  everything for small-item workloads.
- The protocol around it is the real asset: resumable, multi-provider,
  range-set requests (`/iroh-bytes/4`), a crash-consistent store
  (redb + files), HashSeq **collections** (a blob whose content is a
  sequence of child hashes — n0's manifest shape), tickets, and an
  ecosystem. When this research says "Bao today", it means this profile.

### 3.3 Bab / WILLIAM3

Bab is a hash-function *family* designed for exactly this problem —
"a specification for a hash function specifically designed for usage in
peer-to-peer content-addressable storage systems". WILLIAM3 is its
BLAKE3-based instantiation: width 32 B, chunk size 1024 B, and precisely
three differences from BLAKE3 (Bab spec §2.3, confirmed in the bab_rs
source):

1. **No chunk indices.** Where BLAKE3 feeds the chunk counter into the
   chunk chaining value, WILLIAM3 feeds 0. A chunk's label is a function
   of its bytes alone.
2. **Lengths in inner labels.** `hash_inner(left, right, len, is_root)`
   mixes the total length of the subtree's chunks into the parent label
   (via the compression function's `t` input, where BLAKE3 puts 0).
3. **Different constants** (the IV is the BLAKE3 digest of the ASCII
   string `WILLIAM3`), so the two functions never collide by accident.

Consequences, each verifiable from those two changes:

- **Subtree identity.** A node's label depends only on (left label,
  right label, subtree length). Identical content forming a complete
  subtree — anywhere, in any blob — has the same label and is the same
  servable object. This is the seed hypothesis's core claim, and it is
  **true at subtree granularity**: the unit of sharing is a chunk or a
  complete subtree, so a repeated substring shares exactly when it is
  chunk-aligned and subtree-shaped. §5 measures both halves of that
  sentence: unaligned repeats share **zero** chunks; the same corpus
  padded to chunk alignment shares **344 of 1,210 chunks**.
- **Constant-size length proofs.** The root label commits to the total
  length through the root's own `hash_inner` length input. Proving
  "this digest names a string of length L" = present the root's two
  child labels and L; the verifier recomputes one parent hash.
  **72 bytes, independent of blob size** — against Bao's ~1.7–2.3 KB
  right-edge proof and its final-chunk discipline. For D1's
  digest+expected-length references and for snapshot embeds that want
  to state an expected size, this is a qualitative difference, not a
  constant-factor one.
- **Self-locating slices.** Because every inner label on a slice's
  proof path carries its subtree length, the proof itself tells the
  verifier where the slice sits and how long the whole string is —
  no trusted length header, no final-chunk detour. The Bab spec
  (§§3.2, 4) additionally defines streaming optimizations (omitting
  left labels, omitting lower layers, slice streaming) that shrink
  slice traffic below the Bao traversal's constant factor; this
  research did not quantify those and does not claim numbers for them.
- **Position is still bound — by structure, not counter.** §3.5 of the
  spec ("No Chunk Indices") argues the security point: a chunk's place
  is fixed by the path from the root and the length commitments of its
  ancestors, so removing the counter does not let an attacker permute
  chunks; identical chunks are interchangeable precisely because they
  are identical bytes.

**Costs and risks, stated as plainly:**

- **Digest-correctness churn.** willow25's packaging requires pinning
  `bab_rs = 0.8.1` because *every bab_rs version before 0.7 computed
  incorrect WILLIAM3 digests* (recorded in the estate's Willow research
  from the Riot implementation audit). A hash layer whose digests
  changed under it within the last year is a layer whose stored
  addresses may silently rot. This is the single hardest fact against
  adopting Bab today.
- **Implementation youth.** bab_rs's first release was Oct 2025; the
  storage APIs and slice verification landed through early 2026;
  fuzzing found integer overflows (May 2026 diary), the optimized slice
  paths were still broken at one announcement and under fuzz repair
  after. Trajectory is good; the destination is not yet reached.
- **No finished wire protocol.** Bab today has storage + local verified
  streaming (the willow25 persistent store stores payloads via Bab).
  The network protocol that would move Bab payloads between peers —
  WTP — is a pre-implementation draft ("surveying message encodings"),
  and Confidential Sync's rework is sequenced after it. Adopting Bab
  for sync in 2026 means writing that wire protocol ourselves.
- **Orbit.** Two people (worm-blossom), grant-funded, specification-
  first pace, ~$3K/yr donation income. The orbit's posture, per the
  estate's catalogue work, is that LLM-generated contributions are not
  accepted — upstream engagement from this estate would be
  human-written or none. None of this is disqualifying for a *tracked
  successor*; all of it argues against betting the payload layer on it
  this year.
- **Tree storage is not smaller.** A full Bab label store is 32 B ×
  (2n − 1) nodes ≈ 64 B per chunk — the same ~6.25% as vanilla Bao's
  outboard. Bab's win is what labels *mean*, not how many bytes they
  take. (A grouped-leaf profile like bao-tree's is conceivable for a
  Bab instantiation — the family is parameterisable — but WILLIAM3 as
  standardised is 1 KiB chunks, and changing that changes every digest.)

### 3.4 The hash-world collision (a case study in interop gravity)

n0's iroh-willow implemented Willow + Meadowcap over iroh with payloads
in iroh-blobs — which forced instantiating Willow with **BLAKE3**
digests. Result, in the estate's Willow research's words: "it is *a*
Willow, but not Willow'25, and would not interoperate with willow25
peers." The implementation is also unmaintained (built against iroh
0.34-era APIs; downstream vendoring reports broken builds).

Two lessons. First, **the payload hash is an interop boundary**: every
ecosystem you touch has already picked one, and bridging means running
both or converting at the border (a digest translation is a re-hash —
cheap for the *receiving* side to verify only if it also fetches the
bytes; a translated digest is not self-authenticating across worlds).
Second, gravity is real but not destiny: iroh-willow chose the
transport's world over the data model's world because the transport
was the running code. Our estate's running code is likewise
iroh/p2panda-shaped today — which is an argument for Bao *now* and for
keeping the flip *possible*, not for pretending the flip is free.

### 3.5 Per-item addressing + manifest (the Merkle-DAG shape)

Each item is hashed as its own unit (any tree hash — at item sizes of
1–64 KiB the tree is a formality); a manifest names the ordered set:
(digest, length) per item, ~40 B/entry. This is the most deployed
shape in the field under several names: git's tree-of-blobs, IPFS
UnixFS over DAG-PB, iroh-blobs HashSeq collections, ATProto's repo of
record CIDs under an MST.

- **Identity** is intrinsic per item: the same item in a thousand
  collections is one object, stored once, fetched once, verified
  standalone. Transclusion is then not a feature of the store — an
  embed *is* a manifest entry (plus D2's verb/capability semantics on
  top).
- **Versioning cost** is exactly the changed items + the manifest diff,
  independent of where edits land (no boundary shift: items are cut by
  the normalizer, not by byte offsets). Measured: 1.3% for the §5 edit
  cycle; duplicate instances in the corpus (100 of 403) cost **zero**
  extra storage — 298,953 B (~29% of the blob) exists once.
- **Manifest cost** is the honest weakness: a client needs the relevant
  manifest entries before it can name items. Modelled per-item (one
  manifest slice per fetch, uncached), transclusion of 12 items cost
  45.7 KB vs Bao-slices' 51.2 KB — the manifest slices ate most of the
  advantage. With the manifest fetched once (16 KB for 403 entries) or
  cached, the same fetch is ~30 KB and every later fetch is pure item
  bytes. Manifests must themselves be content-addressed, tree'd when
  large (§3.7), and treated as first-class sync citizens — the shape
  iroh-docs already runs (entries reconciled by set reconciliation,
  values fetched as blobs).
- **What it does not solve:** sharing *below* item granularity. A 1-byte
  edit to a 40 KiB item is a new 40 KiB item. Git's answer (packfile
  deltas) works for storage but not for named transfer. If the estate
  grows items that are large *and* internally edited, this layer needs
  help from below (§6, hybrids) — that is the workload that would
  re-open the Bab question from the data side.

### 3.6 Content-defined chunking + manifest (restic / FastCDC shape)

Split the byte string where a rolling hash (Rabin fingerprint in
restic; gear hash in FastCDC) says to — min/avg/max bounds (restic
defaults ≈ 512 KiB / 1 MiB / 8 MiB; this spike used 4/16/64 KiB to fit
a 1 MiB corpus). Chunks are named by their hashes in a manifest/index.

- **The one thing it does uniquely well:** insertion/deletion locality
  in a *monolithic* byte string. Boundaries re-synchronize within a
  chunk or two after an edit, so version updates touch only the edited
  neighbourhood — measured 16% on the §5 edit cycle, vs 89–100% for
  fixed chunking. This is why every backup tool on earth uses it.
- **Why it stays out of the peer protocol anyway:** chunks are
  variable-size and content-cut, so a chunk cannot be located, sliced,
  or length-checked from its digest; a D1 URI cannot name "the chunk
  containing offset X"; verification always routes through a manifest
  that must itself be trusted/fetched; and for data that *has* item
  structure, per-item addressing beats it on its own metric (1.3% vs
  16%) while also providing names. CDC is the right answer to a
  question — "how do I dedup versions of an opaque byte stream?" —
  that our normalized shapes mostly don't ask.

### 3.7 Prolly trees / Merkle search trees (the manifest's grown-up form)

A prolly tree is a Merkle tree built by recursively applying a
content-defined chunker over *sorted keys* (Noms → Dolt); ATProto's MST
is the same idea with key-hash-determined fanout (4, via 2-bit chunks
of the key's SHA-256). Properties that matter here: history
independence (same set → same root regardless of insertion order),
structural sharing between versions, and diff between two versions in
time proportional to the diff. This is not a blob hash and doesn't
compete with §3.1–3.3; it is the candidate format for the *item index*
when collections outgrow flat manifests, and for any future
Willow-adjacent key/value state (Willow's own store is an entry set
with range-based set reconciliation over it — a sibling answer to the
same problem). Named here so D3 reaches for it deliberately instead of
reinventing it.

### 3.8 Willow's own store assumptions (for decision point 5)

If a domain earns the Willow data model, its store story is: entries
(path, subspace, timestamp, payload_digest, payload_length) reconciled
by range-based set reconciliation; payloads fetched by digest with
Bab verifiable streams and stored via Bab in the persistent store —
i.e., Willow'25 *presupposes* §3.3 as its payload layer and inherits
both its strengths (length-committed digests fit the entry format
exactly: digest + length is the whole reference) and its current gaps
(no finished WTP wire protocol; the estate would write the sync, as
#13779 already records). Nothing in this research changes decision
point 5's framing; it prices the payload layer that adopting Willow
would commit us to.

---

## 4. Side-by-side

| | Bao (vanilla) | bao-tree / iroh-blobs | Bab / WILLIAM3 | Per-item + manifest | CDC + manifest |
|---|---|---|---|---|---|
| Digest of blob | BLAKE3 root | BLAKE3 root | WILLIAM3 root label | manifest root over item digests | manifest over chunk digests |
| Sub-blob identity | none (positional) | none (positional) | **subtree labels, position-free** | **per item, intrinsic** | per chunk, content-cut |
| Slice proof (1 chunk, 1 GiB) | 1,288 B | group transfer ≥16 KiB | 1,288 B (spec opts. go lower, unquantified) | item's own small proof | via manifest only |
| Length proof | ~2,312 B + final-chunk rule | same | **72 B, constant** | manifest carries lengths | manifest carries lengths |
| Tree storage overhead | 6.25% | **0.39%** | ~6.25% (label store) | per-item outboards + 40 B/item | ~40 B/chunk of manifest |
| Dedup of repeats | none | none | aligned subtrees only (measured) | **exact, alignment-free** | content-aligned regions |
| Update cost, insertion edit (§5) | 100% | 100% | 89% | **1.3%** | 16% |
| Update cost, in-place edit (§5) | 100% | 100% | **0.3%** | ~item size | ~1 chunk region |
| Wire protocol today | reference impl | **deployed (`/iroh-bytes/4`)** | none finished (WTP draft) | HashSeq over blobs protocol | restic-style, domain-internal |
| Maturity | spec stable, BLAKE3 audited | production-hardened | spec young; impl. pre-1.0, digest churn pre-0.7 | deployed at git/IPFS/ATProto scale | deployed at backup scale |
| Maintenance orbit | BLAKE3 team | n0 (company) | worm-blossom (2 people, grants) | — (a shape, not a project) | restic et al. |

All byte figures from §5's spike or computed from the cited specs'
parameters; treat the table as a summary of the sections, not as
independent evidence.

---

## 5. Measurements

**Method.** A Python model (stdlib only; SHA-256 as the label function —
every reported quantity is byte accounting or op counting, which is
independent of which 32-byte-output strong hash is used) run on the
estate laptop against a deterministic synthetic corpus built to the
brief's three shapes. Corpus: 340 distinct items (1–6 KiB structured
text with shared boilerplate), instanced 403 times into a 1,035,507-byte
collection (100 instances are repeats — the transclusion/shared-item
case), plus one opaque 40 KiB attachment appearing 3 times at unaligned
offsets. Strategies modelled as defined in §3; sync model is digest-set
resync (a replica requests exactly the pieces it lacks, by identity) —
the content-addressed sync model, stated once here and used throughout.
Full code and raw output are supplements to this record
(`spikes/spike.py`, `supplements/spike-results.json`).

**Within-blob dedup.** 1,012 fixed chunks; Bab-distinct labels: 1,012 —
**zero duplicate chunks**, despite 100 repeated items and a 3× repeated
40 KiB attachment, because every repeat sits at a different offset
mod 1024. The same corpus with each unit padded to chunk alignment:
**344 of 1,210 chunks duplicate** under Bab. Per-item addressing on the
unaligned corpus: **298,953 B (~29%) stored once**. *Chunk-layer dedup
is an alignment lottery; item-layer dedup is exact.*

**Scenario A — fetch one 1,812 B item from mid-collection, verified.**
per-item 3,100 B (item + one manifest slice) · Bao whole-blob slice
3,784 B · Bab slice 3,784 B · bao-tree 16,776 B (one 16 KiB group +
proof) · CDC 45,729 B (three ~16 KiB-average chunks). At single-item
scale all tree shapes are tolerable; group granularity is already 9×
the item.

**Scenario B — edit-and-republish** (insert 198 B early; modify one
item everywhere it is instanced; delete one instance; append five
items; v2 = 1,042,776 B). Bytes a v1 replica must receive: **per-item
13,400 (1.3%)** · CDC 170,031 (16%) · Bab 930,136 (89%) · Bao
1,042,776 (100%). *The insertion, not the hash, is what bankrupts
fixed chunking: a 198-byte shift re-cuts every downstream chunk in
both Bao and Bab.* Bab's position-freedom cannot rescue content whose
chunk boundaries moved.

**Scenario B2 — in-place, length-preserving edit only** (one word
swapped inside one item). Bab: **3,072 B** (the three affected chunks —
the other 99.7% of the blob is literally the same pieces). Bao:
1,035,507 B (under digest-set resync, a new root re-names every piece;
see the range-diff caveat in §3.1). CDC: 65,536 B (one max-size chunk
region). *This is the scenario where the seed hypothesis is simply
right — and it is the less common edit shape for normalized data,
because the item layer makes most edits item-local anyway.*

**Scenario C — transclusion fetch.** Fresh client fetches a 2 KiB page
plus the 12 items it embeds (28,194 B of item content): per-item
45,698 B (uncached manifest slices; ~30 KB with the manifest held) ·
Bao/Bab whole-blob slices 51,232 B · bao-tree groups **236,192 B**
(8.4× the content — group rounding on every item). *For
transclusion-heavy rendering, verification granularity is the dominant
cost in the whole study.*

**Scenario D — length proof.** Bab: **72 B at any size**. Bao: 1,672 B
at this corpus's size, 2,312 B at 1 GiB, plus the final-chunk
validation discipline. Per-item/CDC: the manifest states lengths (its
own authenticity is the manifest root's problem).

**Analytic table (from spec parameters, cross-checked by the model).**
Outboard at 1 GiB: vanilla Bao 67.1 MB, bao-tree 4.19 MB. One-chunk
slice proof at 1 GiB: Bao 1,288 B, Bab 1,288 B (before Bab's §4
optimizations). Length proof at 1 GiB: Bao 2,312 B, Bab 72 B.

**What was not measured, and why.** (a) *Hashing throughput*: not
benchmarked — WILLIAM3 is BLAKE3's compression function with different
constants and inputs, so its cost class is BLAKE3's; quoting a laptop
microbenchmark of a Python model would have been theatre. (b) *Bab's
§4 slice optimizations*: spec-level only, no numbers claimed. (c)
*bab_rs itself*: not run — its storage APIs exist, but the estate
question is a design decision, and timing a pre-1.0 alpha's local API
would not have measured anything the decision turns on. (d) The corpus
is synthetic text; media payloads (already-compressed, unaligned,
opaque) behave like the attachment — worse for chunk dedup, unchanged
for per-item.

---

## 6. Hybrids and migration

**The recommended composition is already a hybrid:** per-item
addressing (item layer) over Bao-family payloads (tree layer), names
above both. Two more hybrids worth recording:

- **Bab item store + Bao bulk transfer.** If the §7 triggers fire, the
  flip is: item payloads re-addressed under WILLIAM3 (Bab store serves
  items and slices), while *bulk movement between our own servers*
  keeps using Bao/iroh-blobs or Dragonfly-with-pinned-digest as a
  mover — a Bab item can travel inside a Bao blob exactly as a file
  travels inside a tar: the outer layer is transport, the inner digest
  is the identity. The Dragonfly assessment's shape (digest pinned over
  a signed channel, carrier untrusted) is precisely this pattern, so
  the estate's bulk path survives a payload-layer flip untouched.
- **Bao-per-item now, Bab later — the migration path.** Because items
  are addressed by (algorithm-tagged digest, length), migration is:
  re-hash item payloads under WILLIAM3, publish new manifests alongside
  old, let the signed name pointers swing per item or per collection.
  No graph surgery, no URI grammar change (if recommendation 4 is
  taken), no flag day. What a flip *does* foreclose: digest equality
  with the iroh-blobs world for those payloads (they leave the
  sendme-interop set), and any external system that pinned our BLAKE3
  digests must be re-pointed — which is why external pinning should
  always pin the *name* (signed pointer), not the digest, wherever we
  control the contract.

**What each rejected path would have foreclosed.** Bab-everything now:
forks p2panda-blobs, exits iroh-blobs interop, and bets stored
addresses on a hash whose implementation fixed its digests this year.
Bao-everything with no item layer: transclusion and versioned items
pay §5's 8.4×–100× costs forever. CDC in the protocol: unnameable
chunks in the URI layer, permanently.

**Reopening triggers** (any one reopens §0.2–0.3 for the payload
layer): bab_rs reaches 1.0, or two consecutive releases pass with no
digest-correctness fix; a maintained Bab sync wire protocol exists
(WTP or successor, implemented by someone other than its authors);
a workload appears whose payloads are routinely ≥1 MiB, item-internal,
and edited in place (the shape per-item addressing handles worst and
Bab/CDC handle best); iroh-blobs changes its tree format in a way that
breaks the estate's stored outboards anyway (the gravity argument
weakening); or Willow'25 interop becomes a goal in itself (a domain
earns decision point 5 *and* needs to talk to willow25 peers), which
makes WILLIAM3 payloads mandatory for that domain regardless of this
research.

---

## 7. Risks and honest boundaries

- The spike's sync model (digest-set resync) is the content-addressed
  model; a Bao deployment with a bespoke range-diff client can beat
  its Scenario B number, at the cost of fetching proofs for every
  reused range and giving up third-party piece serving. The
  recommendation does not depend on hiding this: even under a
  range-diff model, Bao cannot *name* a sub-piece, which D1/D2 need.
- Bab's security argument for removing chunk indices is the spec's
  own (§3.5); WILLIAM3 has no independent audit and a fraction of
  BLAKE3's scrutiny. The recommendation's caution is therefore
  two-sided: Bab is both less mature *and* less reviewed.
- All measurements are one synthetic corpus at ~1 MiB. The *ordering*
  of strategies is structural (it follows from alignment and
  granularity, not from the corpus), but the percentages are the
  corpus's, not constants of nature. The supplements carry the code so
  any lane can re-run at other scales.
- Willow/Bab ecosystem facts are as of the estate's Nov 2026 research
  and this lane's spec reads; bab_rs is moving monthly. The §6
  triggers, not this document's snapshot, should drive the flip.

## 8. Sources

Primary: Bao spec (oconnor663/bao, docs/spec.md — combined/outboard/
slice formats, final chunk requirement); Bab spec (worm-blossom,
bab-hash.org / worm-blossom.github.io/bab — §§2–4: parameters,
WILLIAM3's three differences, slice & length verification, no chunk
indices); bab_rs source and docs (docs.rs/bab_rs 0.8.1 — WIDTH 32,
CHUNK_SIZE 1024, hash_inner length input; storage module); Willow'25
parameters (willowprotocol.org — WILLIAM3 payload hash, 32-byte
digests); iroh blob-store design post (iroh.computer — 16 KiB chunk
groups, outboard rationale) and bao-tree docs (BlockSize 2^4);
iroh-blobs survey material in the estate's iroh census (0.103.0,
`/iroh-bytes/4`, HashSeq collections).
Estate records: p2panda-investigation (comparison-and-recommendation
§§5–6; research/willow-meadowcap §§10–12 — iroh-willow hash-world
collision, bab_rs 0.8.1 pin and pre-0.7 digest defect, orbit and
funding; research/p2panda — p2panda-blobs wraps iroh-blobs, operation
header = BLAKE3 hash + size; research/worm-blossom-catalogue and
-questions — Bab diary timeline, WTP pre-implementation status,
contribution posture); issues #13779, #13880, #13885, #13886, #13887,
#13929 (Dragonfly assessment, comment 7119909532).
Classical: FastCDC (USENIX ATC '16) and restic chunker parameters;
prolly trees (Noms/Dolt) and ATProto's MST repository spec (fanout,
history independence, structural sharing).
