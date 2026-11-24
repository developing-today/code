# Worm Blossom catalogue — the collective's entire blog and project set

Research note for the capability-auth + peer-to-peer transport investigation
(developing-today/code#13779). Companion to `willow-meadowcap.md` in this folder,
which covers the Willow/Meadowcap specifications in depth; this note covers the
*people, their public diary, and everything they maintain*, from their own pages.

Compiled 2026-11-23 from primary sources only: worm-blossom.org (front page +
the three archive pages + the All Music / All Comics index pages),
willowprotocol.org, bab-hash.org, sneakerweb.org, the Codeberg organisation
`worm-blossom` (org page snapshot), the JSR scope `@wormblossom`, and docs.rs.

## Counting method and totals

- **Blog: 59 weekly updates.** The blog *is* a weekly diary — there are no
  standalone posts; every update bundles editorials, announcements, a music
  pick, link notes, and a comic. Counted as update-blocks on the site's own
  archive index (https://worm-blossom.org/archive): **2025 = 12 updates**
  (2025-W41–W52, page `archive_2025_2`; the diary launched 2025-W41, there is
  no 2025 part 1), **2026 Part 1 = 26 updates** (2026-W01–W26, page
  `archive_2026_1`), **2026 Part 2 = 21 updates** (2026-W27–W47, page
  `archive_2026_2`). The front page repeats the 8 most recent updates
  (W40–W47); verified duplicate, not additional posts. Week labels come from
  the site's own asset paths (`assets/week/yYYYYwWW/…`); the diary prints no
  dates, so calendar ranges below are the ISO weeks. The diary's own
  first-anniversary editorial says "52 weeks" — their count is looser than
  the archive's block count (some weeks are near-empty); the archive count
  above is the verifiable one.
- **Two index pages** re-collect weekly segments and are not separate posts:
  *All Music* (the playlists pages: every weekly song, plus album playlists)
  and *All Comics* (https://worm-blossom.org/archive/comics/ — 43 comic
  entries: 10 from 2025, 33 from 2026, including the multi-part *Blossoquest*
  serial).
- **Projects: 20 maintained by the collective** (catalogued below), plus
  **3 independent third-party implementations** of their specs (Ourcana's
  Dart Willow, Freckles' willow-ts, a Zig Bab) listed separately because
  they are the strongest external-adoption evidence.

## The collective

Two founders, one later member, grant-funded (NLnet/NGI) throughout:

- **Aljoscha Meyer** — researcher (PhD work on set reconciliation and,
  lately, certificate transparency); author of the Willow specs, Bab,
  Ufotofu, Macromania; signs editorials `~Aljoscha`.
- **Sammy M. Gwilym** ("Sammy", formerly credited as Gwilym) — developer and
  artist; inherited stewardship of Earthstar from its creator Cinnamon;
  co-author of Willow; draws the comics; signs `~sammy`.
- **Emmy Walker** — contributor from early 2026 (ufotofu, sneakerweb; earlier
  editorials bylined "Dan/danmw" are the same person), **officially joined
  as third member in autumn 2026** and works on Pillow.
- Regular community contributors named in the diary: Miaourt (Bab, guest
  editorials), Vivicat (WILLIAM3 debugging), Sarah Aronson/"Heni" (Ourcana),
  Freckles (willow-ts, test-vector auditing), Slug, interfect, Ben Frye.

---

# Part 1 — The blog, week by week

Relevance tags: **[auth]** capability/Meadowcap · **[sync]** sync protocols ·
**[enc]** encryption · **[store]** storage/indexing · **[rust]** Rust library
design · **[eco]** ecosystem/adoption · **[—]** no direct bearing.

## 2025 (archive_2025_2)

### 2025-W41 · Oct 6–12, 2025 — Diary launch; first `bab_rs` release
Aljoscha's launch editorial: after a quiet year accumulating Willow work and
technical debt under grant deadlines, worm-blossom will publish minimally
useful subsets early, clear the backlog publicly, and post joyful weekly
updates. Announcement: first release of `bab_rs`, the Rust implementation of
**Bab**, a BLAKE3-inspired hash-function family designed for content-addressed
storage; fully generic over Bab's parameterisation, shipping **WILLIAM3** as
the recommended default; verified-streaming features planned (funded). Sammy:
two Willow specs renamed — Willow General Purpose Sync → **Willow
Confidential Sync**, Sideloading Protocol → **Willow Drop Format**.
*Relevance: [enc][store] — Bab/WILLIAM3 is the hash layer under Willow'25
payloads and verifiable streams; everything later in the diary builds on it.*

### 2025-W42 · Oct 13–19, 2025 — Ufotofu week; Frugal Async Rust
Aljoscha: heavy **Ufotofu** week (docs, tests, doc-tests toward a proper
release) — their Rust lazy-sequences library (producers/consumers, i.e.
streams/sinks) that "served us well for our async needs while implementing
Willow in Rust"; plus breaking core changes to **Macromania**, their
TypeScript-embedded templating language that generates willowprotocol.org
(core now stable, JSR release, high-level docs missing). Announcement:
**Frugal Async Rust** page published and **`frugal_async` crate released** —
simplifying assumptions for async Rust (an awaitable Mutex example;
smol-based). Sammy's editorial is about cooking.
*Relevance: [rust] — Ufotofu and the frugal-async discipline (no `Send`
bounds, single-threaded futures) shape every later implementation choice,
including a real framework incompatibility in 2026-W42.*

### 2025-W43 · Oct 20–26, 2025 — willowprotocol.org mega-update
Sammy: 18 months of Willow work (Apr 2024–Oct 2025, NLnet-funded) published
at once in a 67,020-additions mega-branch: Confidential Sync reworked to be
**MITM-resistant** (Private Interest Overlap redesigned; a threat-model
page), Drop Format can add entries **without payloads** with a more efficient
encoding, a new **Willow'25** page (the recommended parameter set), a new
Rust section with tutorials and rustdoc integration, front page redesigned.
Miaourt's Bab contributions landed in that week's `willow25` release.
Announcement: Joakim's contribution reinterprets Willow timestamps as **TAI**
(leap seconds), surfacing epoch-choice pitfalls in the Rust `hifitime` crate.
*Relevance: [sync][auth] — the public spec surface (Confidential Sync's
privacy design, Willow'25 parameters) dates from this drop.*

### 2025-W44 · Oct 27–Nov 2, 2025 — First `willow25` crate release
Announcement: first `willow25` crate — the full Willow Data Model with
Willow'25 parameters; highlights: optimised immutable `Path` with zero-copy
prefix creation, `Timestamp` arithmetic, the core `Entry` struct; new Rust
tutorials. Aljoscha: the past year's Rust implementations are being reworked
and re-released over coming weeks; roadmap: Groupings → Meadowcap → storage
traits → in-memory + persistent stores → Drop Format; then slower work on
application ideas, E2EE, network sync; Bab, Ufotofu, Macromania on the side.
Also part 1 of the "why *is* this website called 'worm-blossom'?" series
(French revolutionary decimal time).
*Relevance: [store][rust] — the implementation roadmap they then actually
followed, in order, for a year.*

### 2025-W45 · Nov 3–9, 2025 — `willow25` 0.2.0 groupings; `order_theory`
Announcement: `willow25` 0.2.0 adds the `groupings` module (WillowRange,
Range3d, Area, AreaOfInterest types; a `Grouping` trait so entries can be
checked against any grouping). Aljoscha: **`order_theory` crate** published
"accidentally" — traits for successors/min/max/lattices born from Willow's
needs; its property-testing utilities found several errors in their previous
groupings implementations. Sammy: funding chase, including the **Rummager**
proposal (a general-purpose GUI for "organically expressing your own
applications through the act of nesting free-form data"); new **ufotofu
website**. Aljoscha's essay traces ufotofu's lineage to SSB pull-streams
(Dominic Tarr), xstream/callbag (André Staltz), and Go SSB's streams lib —
his rejected Onward! 2024 stream-API essay became the ufotofu site's basis;
the library has been "simmering for almost ten years."
*Relevance: [auth][store] — Areas/groupings are the unit Meadowcap grants
over; order_theory is the lattice toolkit underneath.*

### 2025-W46 · Nov 10–16, 2025 — "…on 'fancy' CRDTs"; ufotofu codec traits
Publishing day moves Monday→Friday; the **Rummager proposal was rejected**
("we'll just need to build out a prototype in our spare time"). The titled
essay **"…on 'fancy' CRDTs"** argues Willow's last-write-wins choice: fine-
grained merge CRDTs "often require lugging around a complete history of
changes, forever," while most connected apps (microblogging, chat, forums,
media, issue trackers) author data alone then pool it — Willow focuses on
"merging potentially huge collections of interleaved data," a system "we
*know* we can make efficient." Announcement: **ufotofu minor release adding
codec traits** — `Encodable`/`Decodable` plus **relative encodings** (encode
relative to context shared by encoder/decoder → delta encodings); many
Willow encodings rely on this, and ufotofu's codec traits map directly onto
Willow's (relative) encoding relations. Aljoscha details the codec axioms
(every value has an encoding; no two values share one; no encoding is a
prefix of another) and the `codec::proptest` module (`assert_codec`) with
coverage-guided fuzzing, which finds counterexamples in implementations
*and in spec encoding definitions*; Meadowcap's Rust implementation is next.
*Relevance: [sync][rust] — the LWW-over-history argument is the core design
bet Willow makes against log-based systems; relative encodings recur in the
WTP design a year later.*

### 2025-W47 · Nov 17–23, 2025 — Interactive Data Model; `compact_u64`; the semilattice editorial
Sammy built the **Interactive Willow Data Model** (browser toy: view/add
entries, "sync" between three peers) for willowprotocol.org. Announcement:
**`compact_u64` crate released** — the variable-length u64 encoding used
throughout the Willow specs. Aljoscha's rant: **LEB128 is "the linked list
of varints"** — byte-at-a-time decoding with unpredictable branches is
hostile to modern hardware; length-up-front designs (CBOR, his VarU64 spec,
Willow's generalisation) process words at a time and their lexicographic
order coincides with numeric order (helpful for kv-stores). His long
"ivory tower" editorial: op-based CRDTs are mostly ad-hoc commutative
operators dressed up; **state-based CRDTs = semilattices** are the real
structure — irredundant join decompositions, join-decomposition
reconciliation, CALM-theorem reflections (Willow's overwriting entry =
deletion inside a monotone system), and **sublattices as the formal basis
of access control and partial sync — "it is no coincidence… every Willow
Area induces a sublattice"**; also intent-preserving lattices (Pijul).
*Relevance: [auth][sync] — this is the theory under Meadowcap Areas and
under range-based set reconciliation: partial sync and scoped access are
the same sublattice operation.*

### 2025-W48 · Nov 24–30, 2025 — `meadowcap` crate released; the empty-ranges bug
Announcement: **`meadowcap` crate released** — Rust implementation of the
Meadowcap spec: authorise writing entries and whether a peer may request
entries; access scoped to arbitrary Areas; holders can delegate, optionally
restricting to a smaller area. Aljoscha's editorial: the **empty-ranges
bug** — syntactic vs semantic equality of ranges (`[2..2)` vs `[3..3)`)
broke the `EncodeAreaInArea` relative encoding during the Meadowcap port;
normalising would violate Willow's encoding axioms (equal values ⇔ equal
encodings), so **empty groupings were removed from the spec entirely**;
caught by a property test; cost days of refactoring.
*Relevance: [auth] — Meadowcap as running Rust code dates from here (the
crate was later deprecated in favour of willow25's built-in module, see
2026-W33); the bug story is a data point on their spec↔implementation
feedback loop actually working.*

### 2025-W49 · Dec 1–7, 2025 — willow25 gains batteries-included Meadowcap
Announcement: **willow25 gains batteries-included Meadowcap** (ed25519
instantiation; `WriteCapability::new_communal` example producing authorised
entries); "with access control in place, willow25 is ready for… persistent
entry storage." Contribution: danmw added `Path::create_slice`/
`Path::create_suffix` to willow25 (restructured Path internals for O(1)
instead of O(n)). Aljoscha: forking BeepBox→AbyssBox (a fork-of-forks mod)
to build the site's **playlists page** — the origin of the weekly music.
*Relevance: [auth] — capability issuance/verification usable from the main
crate; the communal-namespace entry point is the one an adopter calls.*

### 2025-W50 · Dec 8–14, 2025 — WebTT spec; Bab storage internals
Announcement: **WebTT (Web Tooltips) spec published** — lightweight nestable
cross-domain tooltips for any website with no server-side logic: include a
spec-compliant JS file + CSS, mark elements with `data-tooltip-anchor`
pointing at a tooltip page loaded in an iframe on mouseover; JS on Codeberg;
powers willowprotocol.org's tooltip-heavy specs. Aljoscha: **Bab storage
internals** — pluggable backends via a minimal **`ByteStorage` trait**
(essentially an async `Vec<u8>` with up-front capacity); backends bridged
to Bab by treating the byte array as an **implicit representation of the
Bab Merkle tree** (flat-array tree layout, chunk-data offsets for partial
slices); **`SliceProgressIterator`** emits the Merkle nodes of a verifiable
stream with skip annotations, decoupled from backends, guiding both
producing and consuming; next: consuming/verifying streams into storage.
Sammy: Splintercon (Paris) on the splinternet. Contribution call: willow25
issue #21 (the spec's well-known default keypair enabling public access).
*Relevance: [store][enc] — Bab's storage-first API design (the trait is
tiny; a filesystem backend later took Sammy "next to nothing" to write) is
the piece that makes verifiable partial payloads practical.*

### 2025-W51 · Dec 15–21, 2025 — `willow://` URIs; Drop Format revised; the Cobweb
Announcements: **`willow://` URI scheme spec published** (denotes Willow
Entries and Areas); **Drop Format spec updated** — partial entry payloads
supported (powered by Bab, everything stays verifiable), **drop encryption
removed from the spec** (left to applications; their own apps will default
to encrypted drops), Willow'25 now specifies Drop Format parameters;
persistent storage + drop import/export targeted "within the next two
months." Aljoscha: **the Cobweb** — a hypertext system idea on Willow:
built-in transclusion of multiple documents at once (directory-scale
iframes), Willow URIs → mutability without link rot, extreme simplicity
(markdown-subset markup, one simple image format), efficiency (binary
storage format, mmap-able, interleaved progressive images with
Bab-verified prefixes); inspirations Gopher, Gemini, man pages, RFCs,
Mastodon, TiddlyWiki, Joplin. Sammy: FOSDEM talk accepted ("Willow - A
family of peer-to-peer storage protocols"); implemented a **filesystem
backend for Bab** (ByteStorage over `async-fs` File). Comic: Christmas
"revoking your friends' keys" joke.
*Relevance: [enc][sync] — note the spec-level honesty: drop *encryption*
was removed from the spec rather than hand-waved; encryption is an
application-layer choice in their stack.*

### 2025-W52 · Dec 22–28, 2025 — Maximalism post-mortem; `willow_data_model` 0.6.1; 2026 roadmap
Sammy's year-end editorial, **the maximalism post-mortem**: Confidential
Sync's complexity "radiated onto everything" (storage needed RBSR
fingerprint/partition APIs and push-centric APIs; ufotofu needed a menagerie
of encoding traits for Private Interest Overlap's private encodings); the
Macromania site rewrite congealed into a mega-branch that sealed
willowprotocol.org against small updates; silence fed perfectionism;
lessons: avoid complexity, deliver frequently, focus, communicate often —
worm-blossom.org is the antithesis. Announcement: **`willow_data_model`
0.6.1** — beginning of proper storage: a basic `Store` trait + non-persistent
`MemoryStore`; the trait can't support sync yet (no incremental payload
receiving/verifying — that'll be a subtrait); planned hierarchy of
subtraits (Bab-related methods, summarising for RBSR, …). Aljoscha's **2026
roadmap**: by end of January, simple persistent storage + Drop Format
implementation → sneakernet apps possible; then revisit **E2EE of
entries/payloads** (specify techniques in Willow'25, augment stores to
encrypt/decrypt on the fly, encrypted by default); **pull-based-only
doctrine** (premature push-based APIs — Confidential Sync + subscription
store APIs — blamed for past slowness); next big chunk: **WTP (Willow
Transfer Protocol)** — Confidential Sync is "deeply push-based, so it is
not an option"; WTP is HTTP-inspired simple requests/responses (draft
linked); after WTP: push-based local APIs (subscribable Bab, then
subscribable stores); a more efficient storage backend (currently linear
scans; 3D generalisations of zip-trees and G-trees in mind); long-term: a
**capability-scoped Willow database server** shared by multiple apps, with
persistent subscriptions resumed by offset + set-reconciliation catch-up;
Confidential Sync rework comes last, informed by WTP; plus benchmarking
suites, transactional store APIs, a peer-sampling service, Willow URIs
implementation; application ideas Rummager + Cobweb.
*Relevance: [sync][store][auth] — the single most decision-dense post of
2025: it admits the flagship sync protocol was over-complex, demotes it,
and lays out the pull-based WTP + capability-scoped server direction that
the whole of 2026 then executes (and that Pillow later funds).*

## 2026 Part 1 (archive_2026_1)

### 2026-W01 · Dec 29, 2025–Jan 4, 2026 — Unified media formats (design)
Sammy: rest week, sketching the FOSDEM talk. Aljoscha: designing **unified
simple media formats** for Cobweb/Rummager — one format per media type
(image/audio/video) sharing structure: channels of integer samples,
interleaving with per-channel frequencies (chroma subsampling),
**quantisation as the only lossy step**, LZ + predictor residuals (never
encode samples verbatim), seeking via absolute samples + anti-monotone
binary linking forward-pointers; target ~400 lines of C per format (QOI/QOA
as models, ruled out as-is: no combined lossy+lossless).
*Relevance: [—] mostly; the formats work recurs at W39 (seeking schemes)
and feeds Cobweb/sneakerweb media ambitions.*

### 2026-W02 · Jan 5–11, 2026 — Bab stream verification breakthrough
Sammy: FOSDEM talk in "concept phase." Aljoscha: reflective editorial on a
creativity/coding block, ending with a **breakthrough on how to elegantly
implement verification of incoming data streams in Bab** (details promised
the following week).
*Relevance: [enc] — incremental verification is Bab's core promise;
implementation detail lands over the following weeks.*

### 2026-W03 · Jan 12–18, 2026 — Light week
Mines/extractivism comic (the metaphorical-mines series), music picks.
No substantial technical content.
*Relevance: [—].*

### 2026-W04 · Jan 19–25, 2026 — Pointer week
Sammy: restructuring the FOSDEM talk — not "what Willow is" (the website
does that) but "**why** is Willow like it is? What kind of world made us
feel such a thing was necessary?" Aljoscha's editorial is mostly a pointer
to Bab implementation notes on bab-hash.org.
*Relevance: [—] light; the talk framing signals their positioning:
motivations first, mechanism second.*

### 2026-W05 · Jan 26–Feb 1, 2026 — Bab slice verification lands; Open Collective
Sammy: train to p2p-basel (listening, not speaking); FOSDEM talk retitled
"**Willow - protocols for an uncertain future**." Announcement: **worm-
blossom Open Collective opened** — all work (Willow, Bab, Ufotofu,
Macromania, the site) is funded by public funding + volunteers. Aljoscha:
travelled to p2p-basel with Andreas + Glyph **from p2panda**; **Bab slice
verification is implemented**, close to a simple storage backend storing a
single contiguous slice; the FOSDEM deadline was self-imposed and "silly";
Pijul's Pierre expected at Basel.
*Relevance: [eco] — the p2panda connection is personal and long-standing
(weekly calls, shared events) well before the Pillow collaboration.*

### 2026-W06 · Feb 2–8, 2026 — Site restructure; motto list
Aljoscha runs the update solo (Sammy at FOSDEM prep) with a parody letter
from "Samy Gwylim"; the week's **motto list** includes "being-able-to-
cross-a-moat-full-of-crocodiles-based access control" and "gardeners
rather than architects." Announcement: **site restructure** — front page
now shows only the eight most recent weekly updates; a full-archive page
plus All Music and All Comics pages launched.
*Relevance: [—] (and provenance for this catalogue's counting method).*

### 2026-W07 · Feb 9–15, 2026 — p2p-basel sessions: delay-tolerant UIs, "lazy identity"
Sammy's double report. At p2p-basel she moderated "**delay tolerant UIs**"
(authored data arriving a week later, divorced from context; instant-
delivery UIs don't lift) and "**zero / low identity systems**" (digital
doubles linkable to real people are the risk; her "**lazy identity**"
sketch: random IDs for all data, with the owner able to derive a new ID
provably linked to the original — "identity on demand"; compared to
image-board tripcodes; "it would be interesting to plug such a system into
Willow"). FOSDEM: her talk (opening cutscene, ~50 drawings, music by
Aljoscha; recording exists) got a glowing reception; highlights include
Modal Collective's secure p2p-heart OS talk, seeing **adz of p2panda**,
meeting GNOME's Tobias Bernard; Miaourt + danmw attended. Aljoscha: taking
a break (depressive episode).
*Relevance: [auth] — "lazy identity" is their only identity-system sketch:
deliberately minimal, owner-derived, linkability-on-demand — consistent
with Meadowcap's "no assumptions about what an identity is."*

### 2026-W08 · Feb 16–22, 2026 — Drop Format private encodings; ufotofu error design
Sammy: implementing the **prerequisite encodings for the Drop Format** —
Willow's **private encodings** (items decodable only with a bit of common
knowledge; justified by compactness — squeezing maximal Willow data onto
low-capacity USB keys); porting to "another, hopefully final incarnation
of an Ufotofu-based encoding abstraction"; also pondering Discord's
age-verification dispersal problem (no persuasive p2p answer yet).
Contribution: **danmw added chaining combinators to ufotofu**. Aljoscha:
the **ufotofu error-design discussion** Dan's work triggered — some methods
"irretrievably swallow up their arguments in case of an error"; fix
direction: error cases return the original arguments; resolution: "normal"
APIs requiring `Copy` items + clunkier `Clone` variants; **`Copy` exempts
types from linear typing**, matching his mathematical mental model.
*Relevance: [rust][enc] — private encodings are a compactness (not
confidentiality) mechanism; the error-design entry is a good sample of
their API-design rigour.*

### 2026-W09 · Feb 23–Mar 1, 2026 — Private encodings merged; Bab backend logic done
Sammy: **private path encoding, private area encoding, and private
authorisation token encoding merged into willow_rs `main`** — all past the
fuzz-testing apparatus; immediate goal: save/retrieve Willow data + at
least one exchange path (Drop Format). Aljoscha: **Bab persistent backend
logic in place** (remaining: rearrange public API, document, more tests).
Why-worm-blossom part 4 (Dominic Tarr/SSB lineage; Git precedent).
*Relevance: [auth][enc] — note there is a *private authorisation token
encoding*: Meadowcap tokens have a compact shared-knowledge encoding for
the sneakernet path.*

### 2026-W10 · Mar 2–8, 2026 — Rust type-system limits; "persona" not "identity"
Sammy: Internet Archive Europe HQ visit; **hit a dead end in how well Rust
accommodates Willow's generic/parameterised ambitions** in willow_rs;
started the **Drop Format implementation** (WIP PR) using liberal `todo!`
macros — breadth-first vs her usual depth-first. Aljoscha's editorial,
**the Rust type-system limits piece**: the generic implementation +
non-generic willow25 wrappers (via a `wrapper!` macro, `repr(transparent)`
unsafe) break down for wrappers of *composite* generic types (an
`AuthorisedEntry` = generic Entry + generic AuthorisationToken): the
safety of the unsafe code would depend on how generics are instantiated,
which the compiler cannot enforce — "an unholy abomination"; **decision:
keep the generic codebase for now, but new work provides willow25
(concrete) APIs only; long-term, phase out the generic codebase entirely**;
spec genericity still defended; dual-API docs were a pain anyway.
Why-worm-blossom part 5 (Cinnamon's SSB abuse audit — 76 abuse vectors;
deletion impossible in SSB; Earthstar founded 2020 by dropping immutability:
deletion/editing, subset download in any order, one keypair across devices
= keypair represents a person, not a device; but unlinked sets lose SSB's
easy sync — the puzzle Aljoscha's set-reconciliation answers).
*Relevance: [rust][auth] — the genericity retreat begins here and completes
at W33 (generics stripped, −15k lines); the Earthstar history explains why
deletion + subset sync are Willow's founding requirements.*

### 2026-W11 · Mar 9–15, 2026 — Persistent store ported to Fjall; keypair UX
Sammy: **new persistent store for Willow** — a port of the previous
sled-based persistent store to **Fjall** (translation "pretty
straightforward"); store still missing payload methods (Store trait update
depends on Aljoscha's Bab work). Also Tauri GUI experiments raising the
keypair-UX problem — keypairs "are just not anything like user accounts";
proposes **"persona" instead of "identity"** (a persona proves authorship;
you can have many; attach as much or as little identifying info as you
want); notes others are also building Willow GUIs. Aljoscha: **Bab public
API for (persistent) single-slice-per-string storage with partial
verification** well-documented and nearly releasable; creating a
**Bab-aware store trait in willow25** to unblock Sammy's persistent store +
Drop Format; aside: Brooklyn Zelenka's **bijou64** varint spec is an
interesting variation on his older VARU64.
*Relevance: [store] — the shipped persistent store is Fjall-backed (an
embedded LSM kv-store), not SQLite — a concrete data point for storage-fit
comparisons. Persona-not-identity is their answer to multi-device:
one keypair = one persona, carried across devices by the user.*

### 2026-W12 · Mar 16–22, 2026 — `bab_rs` alpha: verifiable streaming + storage
Announcement: **new `bab_rs` alpha release exposing verifiable streaming
and data storage** — create storage backends for strings of a known hash,
feed data piece by piece; prefix data is added, non-prefix data errors and
can resume later; **explicitly alpha: everything about the k-grouping
optimisation is untested and "very, very likely to be broken"; the
persistent storage backend is also untested; in-memory incremental
verification works**. Sammy: wireframes (Penpot) for a **generic Willow
management UI** — Discord's server UIs map cleanly onto namespaces, but
keypairs/capabilities ("What's a root pass? What's a path?…") are the hard
part; the single-namespace-as-bucket vision; past attempts (drop-in React
component; web mini-OS "CinnamonOS"); current suspicion: the answer is
agreed conventions — a "pattern language." Contributions: **Ben Frye**
added ufotofu adaptors (and found a bug in built-in consumers); **danmw
given full ufotofu repo access + crates.io publishing rights and published
ufotofu 0.10.1 himself** ("bus_factor += 1").
*Relevance: [enc][store] — alpha caveats stated in the release
announcement itself; capability UX ("root pass" language) flagged by the
authors as the unsolved adoption problem.*

### 2026-W13 · Mar 23–29, 2026 — Payload storage via Bab; stateless+untrusted impossibility musing
Sammy: XPUB (Rotterdam experimental publishing) talk report. Technical:
**implemented payload storage in the new Willow persistent store using Bab
— "streaming, verified storage."** Aljoscha: admin/teaching week; filed a
batch of Bab issues; main thinking: **a stateless protocol for mutually
untrusting peers may be impossible** — WTP is stateless and assumes a
trusted peer; Confidential Sync is stateful and doesn't; the
stateless+untrusted quadrant resists because untrusted-peer techniques rely
on establishing shared context and referencing it later, "an inherently
stateful business"; even stateful-client/stateless-server eludes him;
intuition: actually impossible (left to future Aljoscha).
*Relevance: [sync] — a stated open problem that directly shapes WTP's
final shape (it ends up supporting *both* stateful and stateless operation,
W44/W35-36).*

### 2026-W14 · Mar 30–Apr 5, 2026 — "Anti-design" talk; Super Drop Format sketch
Sammy: DWeb virtual meetup lightning talk — storyline: data groupings →
extractive capitalisation → Willow as "a kind of anti-design": **devalue
data — make it illegible, disposable, impose a half-life on it through
mutability and forgetfulness**; persistent store + Drop Format "within
reach"; a **secret project** teased (later revealed: sneakerweb).
Aljoscha: Prague workshop — notes from a session on **the limits of
CRDTs** (promised write-up); nerd-sniped into a "**Super Drop Format**"
sketch: drops storing a 3D search tree so area queries run in logarithmic
time in total entries (or linear in area size); "extremely low priority.
Probably."
*Relevance: [sync] — the anti-design framing (data should decay) is the
philosophical root of traceless deletion and expiry-by-timestamp design.*

### 2026-W15 · Apr 6–12, 2026 — SSB design mantras, kept and rejected
Sammy: Prague aftermath (material banked on sneakernets, p2p UIs,
collaboration); shadow-council comic. Aljoscha: **how SSB still influences
his design work** — mantras kept: *no global singletons* (a dependency on
any global server/network/legal entity/blockchain "is brittle and will be
co-opted"); *against consensus* (with nuance: a minimal, clearly-labelled,
eventually-retirable unchanging foundation is valuable; everywhere
evolution is desirable, no global consensus); *free listening* (listening
opt-in, not opt-out). Rejected SSB patterns: **immutability-by-default**,
the **public follow-graph** (enables spidering/surveillance — "this
tradeoff feels inacceptable to me by now"), and SSB's general
virality/network-effect optimisation, which makes it "unsuitable for
vulnerable users" — Willow may be "fated to being crowded out by projects
that emphasise virality over privacy." Note: danmw contributed nicer skip
+ filter adaptors to ufotofu; Sammy posted a clean version of her DWeb
lightning talk.
*Relevance: [auth][sync] — the clearest statement of their threat model
and non-goals: no global anything, no public social graph, privacy over
growth. Any composition with their work inherits these constraints.*

### 2026-W16 · Apr 13–19, 2026 — `willow25` 0.5.0-alpha.1 PersistentStore; sneakernet theory
Why-worm-blossom part 6 (late 2019: Bamboo append-only logs can't support
multi-device identity; sets are dual to logs; unions of sets are easy where
merging logs isn't; Aljoscha's **set-union algorithm** presented at
p2p-basel 2020; Earthstar needs exactly that sync for mutable unlinked
datasets; he chose his masters-thesis topic — fleshing out the set-union
algorithm — "as a present to Cinnamon"). Sammy: **sneakernets** (from
Prague) — her drop model rebuilt: sneakernets as **homeostatic** (data
pushed out + *need* for data expressed; the overlap is a "hot zone" worth
transporting); users should be able to *query* a drop (random access — the
Super Drop Format nerd-snipe); Aljoscha's framing: **sneakernets are
congestion control without explicit feedback** (a full USB drive forces
choices = throttling; implicit feedback = tracking what you receive from
others to prioritise what you send). Announcement: **`willow25`
0.5.0-alpha.1** — the `storage` module with **`PersistentStore`** for
on-disk Willow data; "an *alpha release*: we expect the persistent store
to be buggy."
*Relevance: [store][sync] — persistent storage exists but is labelled
buggy-alpha by its own authors; the set-union origin story is the
intellectual root of RBSR.*

### 2026-W17 · Apr 20–26, 2026 — Interfaces for p2p applications
Sammy: **rethinking interfaces for p2p applications** (Prague session) —
chat interfaces presume orderly, timely delivery; p2p data may arrive long
after authoring, out of context; every device has a different view (email
clients are the precedent for divergent views); order-collapsing interfaces
(emoji reactions reduce many messages to a count) as a pattern for richer
types; archival/canonicity question (are p2p systems inherently
ahistoric?); emblems: the centralised **linear ledger** vs the p2p
**scattered pinboard**. Also: building the **bab-hash.org website**;
curating the **p2p tent at DWeb Camp**. Aljoscha: no code (life confluence).
*Relevance: [—] for the protocol decision; useful signal that their UX
thinking assumes divergent, stale views as normal.*

### 2026-W18 · Apr 27–May 3, 2026 — bab-hash.org launched
Announcement: **bab-hash.org launched** — a dedicated Bab website ("our
cutest website yet"); `bab_rs` framed as a "batteries-included" API going
beyond digests to persisting Bab-annotated data to disk (as used by
Willow's persistent store). Sammy's design editorial: Bab designed for p2p
data exchange — interrupted transfers, malicious peers, many simultaneous
peers; nobody wants to restart an interrupted download or unwittingly
store/propagate illicit data; site is static, built with Macromania ("We
really need to release it someday"); Sammy announces a short break.
Aljoscha: wrapping up the **Bab grant** — implementing a storage backend
that can store **multiple subslices of a single string and append to them
concurrently** (the website promises parallel downloads from multiple
peers, "so the implementation better catch up"); optimised Bab slice
streaming still to test.
*Relevance: [enc] — Bab's stated design centre: verifiable partial data
(resume, reject malicious slices, multi-source download).*

### 2026-W19 · May 4–10, 2026 — Bab fuzzing; Seasonal Clock sighting
Sammy on break. Aljoscha: making the Bab high-level picture precise —
**fuzzing the Bab implementation, finding integer overflows**,
documenting/asserting invariants to turn low-level failures into semantic
concepts. Project sighting: the **Seasonal Clock** (seasonalclock.org) —
"a worm-blossom project we have not really posted about here before";
Gergely Polonkai is porting it to **Pebble e-paper watches** (via SSB) and
seeks pixel-art help drawing its 24 emoji (commission offered).
*Relevance: [enc] — fuzzing is finding real overflow bugs in Bab at this
stage; maturity evidence, not a scandal — they publish it themselves.*

### 2026-W20 · May 11–17, 2026 — 3D zip-trees fiesta
Aljoscha, three editorials: (1) a **visual guide to zip-trees** + the last
missing algorithms for **3D zip-trees** (Willow's 3D data model needs 3D
data structures; existing multidimensional structures all have problems);
a graphical algorithm notation developed as a pseudocode replacement.
(2) Complexity analysis: in a logarithmic-height tree, queries are
logarithmic; **updates logarithmic on average but worst-case O(n^0.792)**
(master theorem, 3 subproblems of size n/4) — "not *great*, but probably
good enough maybe?… R-trees and their ilk typically just hope for the
best." Definition of **2D zip-trees** (4-ary tree; root = maximal-rank
point; quadrants recurse). (3) **2D skip lists** defined recursively and
shown **isomorphic to 2D zip-trees**; connectivity: a directed path exists
between any two vertices; generalises to higher dimensions.
*Relevance: [store] — the 3D-indexing problem that dominated their storage
roadmap — until Amparore's denormalisation insight (W45) dissolved it.
The honest worst-case bound is characteristic: they publish the ugly
number.*

### 2026-W21 · May 18–24, 2026 — Skip-list correction; G-trees generalised
Sammy returns after surgery (short editorial). Aljoscha: **correction to
W20** — the 2D skip-list's links from top-right/bottom-left sublists back
to the pivot are **not needed**; the variant without them should be the
default (destroys unilateral connectivity but gains a **unique path from
the "least" vertex to any other vertex**); progress generalising **G-trees**
to multiple dimensions; neat instantiation: G-trees over almost-complete
BSTs (history-independent, minimal height, but not clamping-invariant;
maintenance not logarithmic — acceptable since G-nodes are small w.h.p.);
the same generalisation for skip lists yields **skip-skip-lists**
(isomorphic to zip-zip-trees); musing that his PhD should be about data
structures, not append-only-log research he "barely care[s] about anymore."
*Relevance: [store] — public self-correction within a week; the storage
substrate research was still unsettled mid-2026.*

### 2026-W22 · May 25–31, 2026 — Sneakerweb announced; Bab fuzz status
Sammy: **Summer'26 edition of worm-blossom.org launched** (seasonal
redesign); DWeb Camp p2p track planning; both will attend and talk on
Willow, "and there's also something else we'd like to showcase…" —
**sneakerweb announced**: "**sneakerweb** is our new experiment in
peer-to-peer publishing, a squirrelly little sibling to the world wide
web." WWW data travels immediately through countless intermediaries and
lives on always-on servers, addressed by scarce/rented/seizable domain
names; **sneakerweb data travels gradually and directly from user to user**
(over the sneakernet — physically moving media), is stored on user
devices, and **anyone can claim their own (base32-key, "far less
memorable") domain forever**. The CLI will: claim sneakerweb domains,
publish websites to them, serve the sneakerweb to an ordinary web browser,
import/export website data from files obtained from others, and block
unwanted data. "**It's all built on Willow** and we're aiming to demo it
at Dwebcamp!" Aljoscha: **Bab fuzz-test status** — a massive fuzz test
(writes data into storage, interrupts at random points, exercises
optimisation options): **with all optimisations disabled it cannot find
crashes any longer; some optimisations still don't work** and he may have
to revisit the actual verification logic rather than fix off-by-ones.
*Relevance: [eco][enc] — sneakerweb is the flagship Willow application;
Bab's optimised paths were still broken at announcement time (they say
so).*

### 2026-W23 · Jun 1–7, 2026 — First interview: Ourcana (a Dart Willow in production); dripdrop; `bab_rs` 0.6.0
Sammy: sneakerweb front page design — browsing to `sneakerweb.localhost`
shows an overview of your collected sites; rejected a `.sneakerweb`
metadata-file approach as dull/bureaucratic; inspired by webrings and
**Nekoweb's** tile grid — let authors present their own sites (an image,
or an iframe to a `sneakerweb.html` page). **First interview on
worm-blossom.org**: Sarah Aronson (aka Heni, heni.lol) on **Ourcana**
(ourcana.space) — an app for the plural community (journalling/notes across
personality states) built after Simply Plural announced its shutdown;
~**9,000–10,000 users** estimated (no analytics, by privacy design;
installs + Discord); Sarah **wrote her own Willow implementation in Dart**
(Flutter clients on iOS/Android/Windows/Mac/Linux/web; the Rust
implementation runs on always-on server nodes — "Rust… is a *service*.
Flutter and Dart are for *clients*"); she matched the Rust implementation
"over the wire in a bit-perfect way" as her correctness criterion; she
plans to **open-source the Dart Willow submodule, a client implementing
the WTP draft spec**; design driver: communal backup — "your friends can
hold onto things for you" without being able to mutate, tamper with, or
(if encrypted) read them; servers open-source from the start, any client
to any number of servers. Aljoscha: **the Bab fuzz test no longer
crashes** — left label omission was the tricky part (fixed); right label
omission on and "everything *continued working*"; **released as `bab_rs`
0.6.0**; next: a backend storing multiple non-overlapping subslices.
Also: **Drop Format spec revised** (motivated by sneakerweb) — the old
count-prefixed encoding breaks when the store is modified concurrently
during drop creation (counts invalidated → invalid drops, fallible
creation); the new encoding is **streaming-friendly**: each entry/subslice
simply flags whether it is final; upfront counts would also be a DoS
footgun (a malicious drop can claim huge counts to force allocations);
decisive argument (Sarah's observation): without upfront counts, **drops
can be created in a completely streaming fashion — e.g. written straight
to a TCP connection**; Aljoscha coins "**dripdrop**" for this streaming
use — one-directional streaming sync for Willow — and notes it could let
him **remove roughly a third of the WTP spec** (the client-push part that
fits pull-based WTP poorly).
*Relevance: [eco][sync] — the strongest adoption evidence in the whole
diary: an independent implementer running a Willow service for ~10k users,
bit-compatible with the Rust implementation, and her production feedback
changing the Drop Format spec. Also: the WTP's shape was still negotiable
mid-2026.*

### 2026-W24 · Jun 8–14, 2026 — Sneakerweb front page; `bab_rs` 0.6.1; Bab grant concludes
Sammy: DWeb Camp prep; sneakerweb front page implemented with arbitrary
`sneakerweb.html` iframes (sandboxed previews); sneakerweb spec drafting
begun. Aljoscha: **finished `bab_rs` 0.6.1** — adds a storage backend that
can store multiple subslices of a single string; **"that should conclude
our funding for Bab"**; options next: DWeb Camp prep, polishing willow25,
implementing Willow URLs, Macromania, multidimensional zip-trees, writing.
External Bab activity: a **Zig implementation of Bab is in the works by
folks involved with Spritely**; Sarah is contributing **SIMD-optimised
assembly to `bab_rs`** (expected 2–4× hash speedup; shoutout deferred
until merged).
*Relevance: [enc][eco] — Bab's funded development ends here; a second
independent implementation (Zig) appears — which matters at W26.*

### 2026-W25 · Jun 15–21, 2026 — Bab project finished; ufotofu-first Drop Format
Sammy's illustration celebrates finishing the funded Bab project; funding
exhausted, grant applications in the pipeline. Sammy + Dan pairing on the
**Ufotofu-first Willow Drop Format implementation** — drop encodings can
be very large, so the API must not "splosh the entire contents of a store
in memory"; a composable API someone can chain into e.g. an entry filter;
"this has not been easy." Aljoscha: small Drop Format spec tweaks;
**drafted a spec for sneakerweb**; DWeb Camp Willow presentation outline;
a proper writeup of **truncated exponential golomb codes**; decided the
residual encoding for the media formats (open: seeking mechanism, audio
sample predictor); started drafting an introduction to p2p systems design.
*Relevance: [rust][sync] — streaming/composable APIs are a hard requirement
their own application imposed on the library; the Drop Format
implementation is ufotofu-shaped.*

### 2026-W26 · Jun 22–28, 2026 — WILLIAM3 digest bugs found by the Zig implementers; ufotofu equality semantics
Sammy: DWeb Camp events announced (**The Willow Skill Tree** talk,
**Sneakerweb workshop**, demo night market); *sammy's guide to the
sneakerweb* prepared as sneakerweb-only content (never to be published on
the www); sneakerweb hardening grant writing ("whether we can harden the
sneakerweb enough that it becomes useful in adversarial environments").
Aljoscha: **the folks implementing Bab in Zig found a bug in `bab_rs` that
produced incorrect WILLIAM3 digests; then he found more bugs; the
implementation still appears to diverge from BLAKE3 in ways it shouldn't;
no complete fixes published yet.** Also a long ufotofu semantics result
with Dan: producer equality — slurping is *not* fully transparent (it may
report errors item-production wouldn't), so either allow observable
side-effects in `ProducerExt::equals` or make `slurp`/`flush` infallible
(`()` return); infallible flush is unjustifiable (flush ≈ `fsync`; a
silently failing fsync is absurd) → **API unchanged, mental model of
producer equality adjusted instead** ("we are nothing if not pragmatic").
And: two albums finished, to be distributed via sneakerweb only.
*Relevance: [enc] — critical maturity evidence: as of June 2026 the
reference WILLIAM3 implementation produced wrong digests (found by an
independent implementer — the multi-implementation strategy working as
intended). Fixed by 0.8.0 (W28), but any "should we use Bab" decision
must price this history in.*

## 2026 Part 2 (archive_2026_2)

### 2026-W27 · Jun 29–Jul 5, 2026 — Sneakerweb RELEASED
Announcement: **sneakerweb released** — "a small application for storing,
browsing, and sharing static websites without relying on the internet at
all"; install the CLI from sneakerweb.org; worm-blossom will bring
sneakerweb-exclusive sites to DWeb Camp. "One of the most exciting aspects
of Willow-based applications such as the sneakerweb is that there is no
need for a single, global seed from which everything grows." Aljoscha:
first **Willow Drop Format data exchanged and ingested** end-to-end — "I
am storing and reading a site that Sammy created. And it all runs on our
own protocols and software. Willow feels so very real all of a sudden!"
(Thanks to Dan's contributions for the running software.)
*Relevance: [eco] — the flagship application ships; Willow's stack
(data model + Meadowcap + Drop Format + Bab + Fjall store) carries a real
product.*

### 2026-W28 · Jul 6–12, 2026 — `bab_rs` 0.8.0: WILLIAM3 verified; guest articles
Aljoscha: **WILLIAM3 bug fixes** — Vivicat spotted and fixed several bugs;
"our WILLIAM3 implementation produces the same digests as a BLAKE3
implementation when undoing exactly the bits that differ between the two
specs. This gives me high confidence that the newly released **version
0.8.0 of `bab_rs`** provides a WILLIAM3 implementation free of bugs."
Guest comic by Christine Lemmer-Webber (Spritely). **Guest article by Dan**:
demoing sneakerweb at his local hackerspace — expected 90 minutes, ran
three hours; attendees swapped `.snk` files and left with personal sites,
photography, a cocktail recipe, and a vanity-prefix brute-forcer for
sneakerweb domains. **Guest article by Miaourt**: **Wi-Fi Aware** — phones
can advertise/discover services and open direct high-bandwidth links
(Android 17 / iOS 26), which could power Willow-style proximity sharing —
but Apple/Google force a manual system pairing prompt for apps while
exempting their own Find My/AirDrop, which "kinda make[s] our dreamed p2p
wireless internet useless."
*Relevance: [enc][eco] — WILLIAM3's correctness is re-established via
cross-implementation checking (BLAKE3 modulo the spec deltas). The Wi-Fi
Aware piece is their only transport-adjacent writing: platform gatekeeping,
not protocol design, is the blocker they identify for proximity sync.*

### 2026-W29 · Jul 13–19, 2026 — DWeb Camp report
Sammy: sneakerweb at DWeb Camp — demo night market table, a workshop
(participants claimed domains, exported `.snk` files), Aljoscha
coordinating physical distribution by USB key; sneakerwebsites reached
Canada, the US, Argentina, New Zealand, Switzerland, the Netherlands, the
UK, Denmark, and Germany; a "getting started" section added to
sneakerweb.org; the site did the rounds on Lobsters and Hacker News.
Aljoscha: post-camp editorial — the politics of the space (software that
"minimises hierarchy and power-over"); his introduction to p2p systems
design "is going to be far from a neutral technical document."
Contribution call: curated good-first-issues in the sneakerweb tracker;
help wanted with cross-compilation for Windows/macOS builds.
*Relevance: [eco] — real-world sneakernet distribution worked socially,
not just technically; the project has outside contributors and press.*

### 2026-W30 · Jul 20–26, 2026 — Seasonal Clock icons; next phase = networked communication
The Hummingcrow icon set for the **Seasonal Clock** (commissioned by
Gergely Polonkai for the Pebble port; 24 hour-icons at 32px/48px, colour +
1-bit versions, CC-BY-SA-4.0, in Gergely's repo) is showcased. Sammy
(holiday editorial): "**The next phase of Willow will be about exploring
what sits *just* above the extreme latency of the Drop Format and
sneakerweb. So: networked communication.** My plan is to use the sneakerweb
itself to find the limits of comfort… (this is what people call 'drinking
your own champagne')." Guest editorial by Emmy (lino printing; a Rust
code-joke marking her name change from Dan).
*Relevance: [sync] — explicit sequencing: sneakernet first, internet sync
(WTP) second, dogfooded through their own app.*

### 2026-W31 · Jul 27–Aug 2, 2026 — Why-worm-blossom part 8 (Seasonal Clock origin)
The series explains the Seasonal Clock's origin: Sammy's Earthstar apps
(microblogging; *Twodays Crossing*, a roleplaying chatroom whose messages
fade over 48 hours) needed a "low res timestamp" that isn't a number —
fractions of the day as symbols (a seed, a leaf, a tree, a shell);
Cinnamon seized on the idea (spreadsheet), and in early 2021 pulled it out
into the standalone **Seasonal Clock** app during "a golden era for the
Earthstar community."
*Relevance: [—] background; establishes the Seasonal Clock as Cinnamon's
design, stewarded by the collective (see project entry).*

### 2026-W32 · Aug 3–9, 2026 — WTP spec work: keep RequestPut; Broadcast-Based Programming
Aljoscha: published **Broadcast-Based Programming** (on his personal site
aljoscha-meyer.de) — a language-design sketch structuring a PL around
broadcasting information rather than method calls on objects (abandons
object identity; spatial computing via active broadcast). Main work:
**fleshing out the WTP into a full specification**. The WTP has three
request types: `RequestGet` (HTTP GET analogue), `RequestGetMany` (areas
or 3D ranges), `RequestPut` (HTTP PUT analogue). He seriously considered
removing `RequestPut` (pure pull-based read-only protocol) but decided to
**keep it**: otherwise app developers need a separate protocol (dripdrop)
to write entries to a peer, forcing multiplexed/multiple connections —
"complexity that outweighs the minimalism." Next steps listed: rework the
start-of-connection flow (the current draft forces every client to first
prove possession of the secret for its read-capability key — write-only
clients shouldn't be forced into useless signatures or a server challenge);
**remove non-Bab usage** (partial verification is too good to let
implementations skip it — no raw byte-offset payload indexing without Bab
verification metadata); define `GetMany` responses; define encodings;
sketch Rust types for all messages (enums instead of optional-field rules).
Sammy: unified navigation ("mega bar") planned across the worm-blossom /
Willow / Bab / sneakerweb sites.
*Relevance: [sync][auth] — the WTP's connection-start detail matters for
any capability composition: read capabilities are *presented and proven*
at session start in the current draft; write-only flows are being
explicitly designed for.*

### 2026-W33 · Aug 10–16, 2026 — Generics stripped: `willow25` 0.7.0; old crates deprecated
Aljoscha: "**we stripped out all generics from our Rust Willow
implementation.** … The pull request ends up reducing our lines-of-code by
about 15000. We have published the new code as **`willow25` 0.7.0**, and
we have **deprecated the old, generic `willow_data_model` and `meadowcap`
crates**." On WTP: making the protocol **symmetric** (both peers may
request entries) might be another way of removing `RequestPut` — but the
decision is "sufficiently fundamental" that WTP work is **postponed**
until Sammy has time to weigh in. Media formats: sample encoding fully
specified; a "half-specification half-rant" document on magic numbers in
file formats. Sneakerweb: Emmy's "fix-all-the-things" PR heading toward
**sneakerweb 1.2.0** (Chrome/Edge support). Also: the **Invisible Library**
— a zero-technology push-based book-gifting practice (contrast drawn with
blockchain/IPFS library schemes: "that… seems a bit excessive"); the
editorial notes the p2panda crew's trajectory (festival planning →
Hoffnung3000 → p2panda → Toolkitty) as the pull-based-general-framework
path deliberately not taken.
*Relevance: [rust] — a major maturity signal, in both directions: the
codebase is now concrete Willow'25 only (the generic spec-parameterised
library is abandoned), and the canonical crates going forward are
`willow25` alone. Anyone evaluating "the Meadowcap crate" should know it
is deprecated in favour of willow25's built-in support.*

### 2026-W34 · Aug 17–23, 2026 — Thin week
Sammy with the kids (no work attempted); a music note. No technical
content.
*Relevance: [—].*

### 2026-W35 · Aug 24–30, 2026 — PILLOW: the p2panda × worm-blossom collaboration; WTP encoding realisations
Sammy's announcement editorial: in March, at a hotel bar with **adz,
glyph, and sam from p2panda**, they drafted a joint grant proposal hours
before its deadline. "**Pillow is a new collaboration between p2panda and
worm-blossom. It's a way for p2panda apps to quickly store and retrieve
data from highly available peers. These highly available peers don't know
anything about p2panda data, but they do have decentralised,
capability-powered permissions, can happily evict stale data, and can
efficiently sync with each other. That's because they're Willow peers!**
In this grant we'll be implementing **WTP**, meaning that at long last
Willow peers will be able to sync over the internet via range-based set
reconciliation. We'll also be developing WTP-powered servers with web
interfaces for configuring capabilities, data eviction, and viewing
storage usage. The first users of these Willow 'support peers' will be
p2panda apps, which will be able to interface with them via Pillow… we'll
also be bringing a new member into worm-blossom." Aljoscha's double
editorial on **WTP message encodings**: stateless requests (responses
fully self-contained — and therefore indistinguishable from eagerly pushed
messages) vs stateful requests (request IDs + relative encodings, e.g.
omitting the namespace from a response); **cancellation** semantics —
no sensible cancellation for stateless requests; stateful cancellation
must normally be confirmed before state can be freed, *but*
length-prefixing responses lets a receiver discard an unwanted response
and keep decoding, giving immediate non-interactive cancellation. Then the
follow-up: `ResponseGet` and `RequestPut` carry the same information
(absolute vs relative encoding) and can **merge into a single message
type**; making as many messages as possible admit both absolute and
relative encodings enables **both symmetric and asymmetric RBSR** (the old
design was client-driven asymmetric only). Contrast drawn with
Confidential Sync (mandatory stateful encodings, mandatory cooperative
cancellation) — WTP is explicitly designed to do better.
*Relevance: [sync][auth][eco] — the single most decision-relevant post of
2026. p2panda — the project this estate is leaning toward — has itself
chosen Willow peers as its availability layer, with Meadowcap as the
permission system for oblivious storage. That is the composed stack
(p2panda data plane + Willow support peers) being built by the two teams
jointly, with WTP as the wire protocol. It also confirms WTP's design is
still moving (message types merged this week).*

### 2026-W36 · Aug 31–Sep 6, 2026 — "Support peers are not servers"; test vectors; the big todo list
Sammy's essay defending support peers against "that's just another name
for a server": servers are high-uptime but **not fungible** (a Mastodon
instance being down can't be substituted); peers are fungible but
intermittent (mobile foreground-only); **a fungible machine with high
uptime is "relatively unexplored"** — a swarm of support peers, possibly
constrained (low storage → prioritise recent data), syncing with each
other. Includes the Pillow↔p2panda bridging diagrams. Aljoscha: work on
the **Willow test vector repository** (encoding vectors updated — some had
been outdated and "actively misleading" — plus new data-model test data:
entry-overwrite determination, path lexicographic successor). His
overwhelm todo list is a map of open work: WTP design (proofs of knowing
the secret for a write capability; unifying request messages; feature
pre-announcement; response mapping); the **ed25519 ambiguity issue of
willow25** (filed on GitHub); updating the ed25519 implementation;
"mildly panicking about quantum computers breaking ed25519"; **store
traits for aggregating metadata (for RBSR) and splitting 3D ranges into
roughly-equal chunks (again for RBSR)**; a database file format for 3D
Willow entry storage + prerequisite 3D-zip-tree work; **recommended ways
of encrypting Willow data (and metadata) at rest as part of Willow'25 —
"we already have generic designs, but no specific instantiation for
Willow25 yet"**; API/implementation designs for that encryption; encrypted
sneakerweb sites (backwards-compatible, ideally); a Willow-powered email
alternative sketch + its encryption issues; Macromania website/macros;
more test vectors for Willow and Bab.
*Relevance: [enc][store][auth] — straight from the author's list:
at-rest encryption for Willow'25 is *designed generically, not yet
instantiated*; RBSR store traits are the active implementation frontier;
ed25519 has both an open ambiguity issue and acknowledged post-quantum
exposure.*

### 2026-W37 · Sep 7–13, 2026 — Test vectors catch a serious capability-verification bug; willow-ts
Contribution notes: **Freckles**, scrutinising the test vectors because
they are writing **a TypeScript implementation of Willow** (browser
Willow), "found a rather serious missing check in our capability
verification implementation" — plus further bugs in specs and
implementation, all reported and fixed through the test-vector process.
**Slug contributed a `sneakerweb` command** (locally removing sites
without blocking them; merged into fix-all-the-things). Sammy: the
"mega bar" navigation is live across the worm-blossom sites; **new Willow
tutorials being written now that the Store and Drop Format APIs have been
published** — writing them surfaces beginner-viewpoint design flaws.
*Relevance: [auth][eco] — two-sided evidence: the capability verifier had
a serious missing check (found late 2026, by an external implementer),
*and* their test-vector programme is exactly what caught it. A third
implementation (TypeScript) is underway.*

### 2026-W38 · Sep 14–20, 2026 — WTP cast into Rust types; media seeking schemes
Sammy: BYOW webcraft meetup report (Rotterdam). Aljoscha: media-format
**seeking schemes** — two designs for logarithmic segment seeking with
single-pass O(1)-memory encoding: a backwards skiplist over segments
(each segment links to predecessor + a power-of-two skip target), and a
**self-synchronising codes** scheme (zero-byte-prefixed segment indexes,
guess-and-scan seeking). Surfer link: **willow-ts** — "A new TypeScript
implementation of the Willow'25 spec. Not quite production ready."
Aljoscha (second editorial): "I took all my pondering over the WTP design
and cast it into a single ~~specification~~ **collection of Rust types**
for the different message types… I hope I can gather some feedback (from
Heni in particular :3)" — i.e. the Ourcana/Dart implementer is a design
reviewer for WTP. He hopes "the WTP design will stay mostly stable" now.
*Relevance: [sync] — WTP exists as Rust types awaiting feedback, not yet
as a wire encoding or implementation; the design review loop includes the
one production implementer.*

### 2026-W39 · Sep 21–27, 2026 — Thin technical week
Sammy: LLM/chat-interface editorial (Kate Bush's *Deeper Understanding*);
contributed content from meetup attendees. Aljoscha: nothing to report
(vacation/cold) + a programming-language design sketch (interface-only
type system; equality from publicly observable behaviour only).
*Relevance: [—].*

### 2026-W40 · Sep 28–Oct 4, 2026 — Sneakerweb 1.2.0 released
Announcement: **sneakerweb 1.2.0** — importantly, collections can now be
browsed from **Chromium and WebKit browsers (Chrome, Safari)**: the
problem was the **base16 encoding previously used for domains**, which
produced hostnames too long; **domains now use base32** (old base16 links
still work in some browsers). Also added: **`sneakerweb remove`** (remove
a domain + its data without blocking it — "an extra level of nuance to
curating your collection"); `serve`/`publish` respect **custom collection
locations** (independent collections; "a stopgap to more complete
collection organisation"). Downloads at sneakerweb.org; full changelog on
Codeberg; thanks to Emmy, slug, and interfect for contributions.
*Relevance: [eco] — a shipped, versioned product with a changelog and
outside contributors; the base16→base32 fix is a real-world encoding
lesson (hostname length limits) from production use.*

### 2026-W41 · Oct 5–11, 2026 — Store tutorial published; Pillow support-node work begins
Announcement: **a new Store tutorial on willowprotocol.org** — willow25's
storage APIs (released two months earlier; "the same APIs we use to power
sneakerweb") now have a walkthrough: instantiate a store, insert an entry,
get it back, append a payload, query areas. Sammy describes the tutorial
pipeline (a `willow-tutorials` repo holding runnable tutorial code;
rustdoc JSON dumped and committed to the willowprotocol.org repo, whose
macro turns API references into docs.rs links). Sammy's main work: **the
Pillow support nodes** — "configurable Willow peers which can communicate
with any other peer which understands WTP. You'll be able to configure
them with capabilities (so that they know what to sync), set storage
limits (so that your Pi with 4gb of storage can pitch in), and configure
if it should sync with other support nodes too. So it's got to be a little
TCP server which can field WTP requests, *and* a HTTP server which serves
up a web UI for administration." Framework research: she likes **Rocket**'s
shape and plans to practise by refactoring sneakerweb's (single-threaded,
"very relaxed pace") HTTP server to Rocket first — then use it for Pillow.
Also: owned vs communal Meadowcap namespaces explainer still being worked
on ("it's about describing the most powerful capability each kind can
have"). Aljoscha: began the **WTP specification** proper (nothing online
yet); **Cardboard** recalled — his older spec for papers-as-static-websites
bundled with their transitive citations; sneakerweb enables the same thing;
proposes **sneakerdep**: a `/sneakerdeps.json` per site listing depended-on
domains, enabling transitive extraction of a self-contained collection and
lazy/on-demand partial downloads of large "papers."
*Relevance: [sync][auth][eco] — Pillow's support node is, in estate terms,
a capability-configured always-on relay/store peer with an admin UI —
the closest existing analogue to the "dumb relay" tier in composed-stack
designs, and it is being built now, not vapourware-roadmapped.*

### 2026-W42 · Oct 12–18, 2026 — `ufotofu` 0.12.7 (tokio wrappers); Rocket experiment FAILS on frugal async
Announcement: **ufotofu 0.12.7 released**, featuring **compatibility
wrappers for the tokio async runtime** — "Now you can use, say, a
tokio-provided TCP stream with ufotofu." Sammy: the Rocket experiment's
verdict — "**sneakerweb cannot work with Rocket.** We can get ufotofu and
Willow'25 working with tokio, but only with the `current_thread`
(i.e. single-threaded) flavour of tokio. Rocket requires a multi-threaded
environment. Ufotofu and willow25's incompatibility with a multi-threaded
environment is due to our adherence to **Frugal Async Rust**." She wonders
whether to be concerned "that Willow and Ufotofu *can't* work in
multi-threaded runtimes, or whether I happened to stumble into the one
situation (Rocket) where this was a showstopper." Pivot: **Axum** (works
with single-threaded tokio) for sneakerweb and, by extension, the Pillow
admin UI. Aljoscha: more WTP spec progress (not yet shareable); a call
with Olivia of **wobbleweb** brainstorming wobbleweb×sneakerweb
integration ("aesthetic sovereignty"); Macromania work — the new
**macromania-web** package (URL references across domains, asset
moving/transformation, automatic stylesheet/script dependency injection;
KaTeX now renders on worm-blossom.org "in a principled way").
Why-worm-blossom penultimate instalment: Sammy inherits Earthstar
stewardship from Cinnamon (2022); Earthstar's sync was "theoretically the
worst possible implementation imaginable: two peers exchange *everything*
they have"; Aljoscha recruited for set reconciliation.
*Relevance: [rust] — a hard, named constraint for any adopter: the whole
Willow/ufotofu stack is single-threaded-tokio only (`current_thread`),
by deliberate design (no `Send` futures). Compositions must budget for
that (e.g. a dedicated thread/runtime per peer), and the authors
themselves flag it as an open worry.*

### 2026-W43 · Oct 19–25, 2026 — Emmy joins worm-blossom; first anniversary
Announcement: "**Emmy Walker is officially joining as worm-blossom's third
member.** Emmy caught our attention with their high quality contributions
to Ufotofu, and has been a driving force behind sneakerweb. Emmy will
immediately be joining the Pillow project." Sammy's anniversary editorial:
"Since then we've posted **52 weeks of editorials**, musical compositions,
and comics. We even released some software." Emmy's forward list: ufotofu
guides ("how to" and "why to") + API creature comforts; making ufotofu
"weirder" (nested producers/consumers); "**working on the Willow'25 side
of the Pillow project, including implementing WTP**"; a
**`SummarisableStorage` trait** "coming to a Codeberg repo near you very
soon." Aljoscha: anniversary reflections (the original premise — publish
useful incremental work — vs the "pretty mad and unsustainable dash" of
the first three months; Macromania is "of all our tooling the one we are
currently using the most" yet least progressed); **multidimensional
G-trees** definition (generalising 2D zip-trees to 2D G-trees; insertion/
deletion "left as an exercise to the reader"). Final why-worm-blossom
instalment: the name is a Seasonal Clock time ("speak to you at
*worm-blossom*"); the weekly p2p call circle includes Sam Andreae and
glyph of p2panda; Sammy + Aljoscha built a TypeScript RBSR implementation
for Earthstar (Sammy wrote a *monoid skip list*).
*Relevance: [eco] — team of three now, with the third member assigned to
WTP implementation for Pillow; the SummarisableStorage trait is the RBSR
store API landing.*

### 2026-W44 · Oct 26–Nov 1, 2026 — New WTP specification uploaded
Announcement: "**We have uploaded a new version of the Willow Transport
Protocol specification.** We will start implementing things based on this
spec, so any and all feedback is highly welcome. The WTP is a protocol by
which peers can exchange Willow data. It supports both **pull-based** (one
peer issues a request, the other responds) and **push-based** (one peer
issues a *long-lived* request, and the other can continuously send new
matching data over time) communication. It can **resume interrupted
payload transfers through the power of Bab**. It can do **efficient set
reconciliation**. It supports both **stateful or stateless operation**.
Oh, and **it doesn't have an actual on-the-wire encoding yet.** We will
start working on that after there's been a bit of time for feedback."
New column launched: **Auntie Source** (agony-aunt for open source).
*Relevance: [sync] — the definitive WTP status: spec (sans wire encoding)
published and declared implementable-for-feedback; push subscriptions are
back in (as long-lived requests) despite the 2025 pull-based doctrine —
the doctrine yielded to a cleaner formulation.*

### 2026-W45 · Nov 2–8, 2026 — Amparore's denormalisation: AreaSlices dissolve the 3D storage problem
Aljoscha: an email from **Elvio Amparore** ("Some observations on Willow
storage and RBSR," building on his 2026 papers on range-based set
reconciliation via range-summarisable order-statistics stores) "was so
impressive that we immediately started reworking the WTP spec and several
pages of Willow docs. Put simply: **it is possible to efficiently answer
arbitrary Area queries *without multidimensional data structures*.**" The
idea: **denormalisation** — materialise additional one-dimensional indexes
instead of one multidimensional structure. Per-subspace 2D trees
(timestamp × path) plus an all-subspaces tree reduce 3D→2D; a further
per-path-prefix denormalisation reduces to 1D trees (each entry indexed in
as many trees as its path has components), and every Area query consults
exactly one index. Generalised: **AreaSlices** — groupings that can be
split into evenly-sized subslices, still served by the denormalised
indexes, and convertible into a small number of normal 3D ranges (so
traditional 3D backends can serve them too). "**We have a definition of
AreaSlices. And the new WTP spec does RBSR in terms of AreaSlices
already. Our old 3dRange type is effectively deprecated.**" The payoff
they name: the Areas-over-arbitrary-ranges restriction, chosen years
earlier, is exactly what makes this possible. Sammy: two `.snk` deliveries
by train (the second-edition guide); sneakerweb **axum branch** underway
(partly as Pillow admin-UI practice); Frugal Async vs multi-threaded
runtimes friction again ("five hours of coding and then running face-first
into a brick wall of inscrutable Rust compiler errors concerning the
`Send` trait"), but reconcilable. Emmy: **SummarisableStore** work
progressing on the Amparore clarity.
*Relevance: [store][sync] — the estate-relevant consequence: a Willow
store no longer needs an exotic 3D index; denormalised 1D/2D indexes
(a design any SQL/kv backend can express) suffice for Area queries and
RBSR. This materially lowers the cost of a libsql/SQLite Willow store —
the gap the earlier research note flagged.*

### 2026-W46 · Nov 9–15, 2026 — Prefix pruning vs Meadowcap scoping; LLM policy; trailing empty components
Sammy's editorial (from a community report by **Alex Burdiyan**): **the
prefix-pruning vs scoped-capability tension** — if you hold read
permission only for `/blog/summer`, a write to `/blog` prefix-prunes your
data, but Meadowcap "really doesn't let you learn about something outside
of the granted area," so the deletion cannot propagate to you without
breaking the scoping rule. Resolution: "**we can let peers determine
whether to reveal pruning entries at their own discretion during sync**"
— no spec change needed. Also a **spec-status policy change**: statuses
are Draft, Proposal, Candidate, Final; "previously we promoted the Willow
Data Model and Meadowcap to final after implementing them, but **in the
future we'll only be doing so after these implementations have seen
meaningful real world usage**." Announcement: **LLM policy** — worm-blossom
projects "won't accept contributions containing output from LLMs"
(formalised on Codeberg). Emmy's editorial: the **trailing empty component
convention** — store data at `/some/path/<empty>` so that adjusting
`/blog/summer` doesn't prefix-prune `/blog/summer/assets`; writes carry a
trailing empty component, deletions don't (empty payload), so entries
split into two flavours: writes and deletions. Sharing signed deletions
out-of-scope still leaks *who/when/at what prefix*, but no payload
information — making the W46 pruning-disclosure trade-off easier to reason
about. Aljoscha: Macromania **defref** ported with **cross-domain
references** (via WebTT) — worm-blossom.org can now define-discuss
AreaSlices although they're defined on willowprotocol.org.
*Relevance: [auth] — a genuine, newly-surfaced (Nov 2026) semantic gap
between deletion propagation and read scoping in Meadowcap/Willow, with
the authors' chosen mitigation (peer-discretionary disclosure) rather than
a protocol fix. Also note: the "Final" status of the Data Model and
Meadowcap specs was granted on implementation, not usage — by the
authors' own new standard, treat both as less battle-tested than the
label suggests.*

### 2026-W47 · Nov 16–22, 2026 — Sneakerweb axum release (speed + big sites); SummarisableStore done; WTP encodings nearly done
Announcement: **a new sneakerweb release focused on speed and big sites**
— sites served "as fast as locally stored content should be served";
**very large files (video) served/scrubbed/seeked via the `video`
element**; sites' total byte sizes shown on the front page; collection
orderable by size; responsive front-page layout; progress indicator for
`sneakerweb publish`; "made possible by **replacing our homebrew web
server with axum**, with Emmy interfacing axum to the Willow store"
(Emmy built **ufotofu↔axum streaming-body adapters**; a ~150 MB-per-episode
BBC series sneakersite as the test case). Also: Aljoscha's album *Computer
Has No Feelings* on Bandcamp (the sneakerweb version's track 4 differs — a
cover, replaced on Bandcamp for legal reasons). Emmy: **store summarising
finished** — "These summaries are one of two ingredients we need to
perform **region-based set reconciliation**… The remaining ingredient is
the ability to split the contents of a store into evenly-sized chunks…
Once we can do both of these things, we'll be well on our way to
implementing the WTP, and having Willow peers that synchronise just the
data they want and need, over the internet." And her **splittable storage**
work (AreaSlices split "more or less by construction" thanks to Amparore);
the wrinkle: "most recent N entries" subsets can only be reported
best-effort, which simplified the summarising API. Sammy: sneakerweb
site-size "weights" (green 0–10 MB, orange 10–100 MB, red >100 MB);
*Sammy's guide to the sneakerweb* 2nd edition now a downloadable `.snk`
starter pack; sneakerweb funding proposals sent; two in-person workshop
proposals accepted; the Safari 30-second-request mystery (base32 hostnames
→ localhost + connection handling); the **willowprotocol.org outage**
(Deno Deploy credits exhausted → six hours down for a static site; site
redeployed and **its repo moved to Codeberg**). Aljoscha: "getting close
to being done with **WTP encoding designs**" (wrists recovering).
Link note: "A Critique of GPUI" (blog.zortax.de) shared by Sammy with a
brief comment (loved Elm; lack of component-level state was fiddly; lack
of a component model makes render diffs harder) — a shared link + comment,
not a worm-blossom position paper. Upcoming: **sneakmeet @ v2 Rotterdam,
13 Dec 26**.
*Relevance: [sync][store][eco] — as of the latest update: store
summarising (RBSR ingredient 1 of 2) is done, splitting is underway, WTP
encodings are nearly designed. Internet sync is the active frontier, not
a finished feature — plan any adoption accordingly.*

---

# Part 2 — The projects

Ordered by decision-relevance. "Depends on it" lists what the diary itself
attests. Versions/dates are as announced in the diary or shown on the
project's own page.

## 1. Willow — the protocol family (willowprotocol.org)
**What:** A family of peer-to-peer storage protocols: the **Data Model**
(entries = subspace × path × timestamp + payload digest; state-based CRDT
join-semilattice; last-writer-wins per (subspace, path) with prefix
pruning = traceless deletion), **Meadowcap** (capability authorisation),
**Confidential Sync** (private, MITM-resistant sync — now demoted, see
below), **Drop Format** (single-file sneakernet transport), **WTP — Willow
Transfer Protocol** (the new network sync protocol), **Willow'25** (the
recommended parameter instantiation: ed25519, WILLIAM3, TAI timestamps),
the **`willow://` URI scheme**, and **AreaSlices** (the RBSR grouping,
late 2026). Spec statuses (their ladder: Draft → Proposal → Candidate →
Final): Data Model and Meadowcap were promoted to **Final** after
implementation — under the policy announced 2026-W46, future promotions
require meaningful real-world usage first.
**Maturity:** Specs mature and versioned; implementations mid-flight (see
willow25). Confidential Sync is effectively superseded by WTP before ever
shipping broadly — the 2025-W52 post-mortem calls its complexity the
collective's biggest mistake.
**Depends on it:** sneakerweb, Pillow support peers, Ourcana, Earthstar 11
(per the earlier research note), the Interactive Data Model toy.
**Relevance:** The family's centre of gravity. The honest internal history
(maximalism post-mortem, pull-based doctrine, WTP replacing Confidential
Sync) is documented in the diary, not hidden.

## 2. Meadowcap (spec)
**What:** The capability system: receiver-bound signed delegation chains
over Areas (subspace × path-prefix × time-range), communal vs owned
namespaces, self-certifying write tokens; read enforcement is cooperative
(sync layer). Full analysis in `willow-meadowcap.md`.
**Maturity:** Spec Final (see the status-policy caveat above); Rust
implementation folded into willow25 (2025-W49) after the standalone crate
(2025-W48) was deprecated (2026-W33). Known open edges from the diary:
no revocation (by design — time-bounded grants + supersession), the
2026-W46 pruning-vs-scoping tension (mitigated by peer-discretionary
disclosure), and a **serious missing check found in the capability
verification implementation in 2026-W37** via test vectors (fixed).
**Depends on it:** Willow sync/drop authorisation everywhere; Pillow's
"decentralised, capability-powered permissions" are Meadowcap.
**Relevance:** The estate's candidate capability layer; the diary adds
production-hardening evidence (and caveats) the spec alone doesn't show.

## 3. willow_rs / `willow25` (Rust implementation)
**What:** The reference Rust implementation of the Willow specs. Current
shape (post-2026-W33): a single concrete crate **`willow25`** (Willow'25
parameters only) with data model, groupings, Meadowcap, Drop Format, and
storage (`MemoryStore`, Fjall-backed `PersistentStore`). The formerly
generic `willow_data_model` and `meadowcap` crates are **deprecated**.
**Maturity:** Active weekly development. Milestones: 0.2.0 groupings
(2025-W45), storage trait + MemoryStore in `willow_data_model` 0.6.1
(2025-W52), `PersistentStore` 0.5.0-alpha.1 ("we expect the persistent
store to be buggy," 2026-W16), **0.7.0** generics stripped (−15,000 lines,
2026-W33), storage APIs powering sneakerweb + Store tutorial (2026-W41),
SummarisableStore (RBSR) landing (2026-W47). Constraints: **tokio
`current_thread` only** (frugal async — no multi-threaded runtimes);
**no SQLite/libsql backend** (Fjall instead); WTP not yet implemented
(encodings in design, 2026-W47). Test vectors repo maintained and used by
external implementers.
**Depends on it:** sneakerweb, Pillow (in progress), Ourcana's server
nodes.
**Relevance:** Adopting Willow in Rust today means adopting willow25's
concrete, single-threaded, Fjall-backed shape — or writing your own store
backend against its traits (which Amparore's denormalisation result makes
much cheaper than it looked a year ago).

## 4. Bab / `bab_rs` (+ bab-hash.org)
**What:** A **family of hash functions** inspired by BLAKE3 and Bao, with
"judicious tweaks": APIs designed from a storage rather than a hashing
perspective, constant-size length proofs, a parameterisable specification,
and optimised computations for repeating substrings. Bab is about
**verifiable partial data**: make progress on interrupted connections,
detect malicious partial transfers, download concurrently from multiple
sources. **WILLIAM3** is the recommended parameterisation (Willow'25's
payload digest). `bab_rs` provides the generic implementation + WILLIAM3,
plus storage backends over a minimal `ByteStorage` trait (in-memory,
filesystem) supporting verifiable streaming and multi-subslice storage.
**Maturity:** Funded development **concluded** (2026-W24/W25). Releases:
first release 2025-W41; alpha verifiable streaming 2026-W12 (k-grouping
optimisation "very, very likely to be broken" at the time); 0.6.0 fuzz-
clean (2026-W23); 0.6.1 multi-subslice backend (2026-W24); **0.8.0
(2026-W28) — WILLIAM3 digests verified equal to BLAKE3 modulo the spec
differences**, after the Zig implementers and Vivicat found digest bugs
(2026-W26). SIMD assembly contributed by Sarah (2–4×, pending merge at
last report). A **Zig implementation** exists (Spritely-adjacent authors).
**Depends on it:** Willow payload verification everywhere (Drop Format
partial payloads, WTP resumable transfers, the persistent store's
"streaming, verified storage").
**Relevance:** Bab vs BLAKE3 is a real trade: BLAKE3 alone gives you
verified streaming via Bao, but Bab adds the storage-first API,
parameterisation, constant-size length proofs, and repeating-substring
optimisation — at the cost of a young implementation whose digest bugs
were only flushed out in mid-2026 by cross-implementation checking. The
0.8.0 verification approach (diff against BLAKE3 with the spec deltas
undone) is the right kind of evidence; there is still no independent
audit.

## 5. Ufotofu
**What:** A Rust library of abstractions for lazily producing/consuming
sequences — **producers and consumers** (their streams/sinks), designed
around strict producer/consumer **duality**: `Producer`/`Consumer`,
`BufferedProducer`/`BufferedConsumer`, `BulkProducer`/`BulkConsumer` for
zero-copy slice processing, `pipe`/`bulk_pipe`, plus **codec traits**
(`Encodable`/`Decodable` with **relative encodings**) that map directly
onto Willow's encoding relations, with a proptest harness (`assert_codec`)
that has found bugs in spec encodings. Futures are **not cancellation-
safe** and carry **no `Send` bound** (single-threaded runtimes only) —
deliberate "frugal async" design. Site: worm-blossom.org/ufotofu/.
**Maturity:** The most mature code they ship: a decade of simmering,
releases through **0.12.7** (2026-W42, tokio compatibility wrappers);
external contributors with publishing rights (danmw/Emmy published 0.10.1);
Emmy is writing "how to / why to" guides (2026-W43).
**Depends on it:** willow25's encodings and Drop Format implementation;
sneakerweb (ufotofu↔axum streaming adapters); historically the reason
Willow's async code looks the way it does.
**Relevance:** If the estate adopts any worm-blossom code first, this is
the lowest-risk piece — and the `Send`-lessness is the constraint that
propagates up the whole stack (see the Rocket failure, 2026-W42).

## 6. Sneakerweb (+ sneakerweb.org)
**What:** "A parallel web transported by physical media" — a p2p protocol
+ CLI for web publishing without DNS, registrars, or hosts. Claim a domain
(a base32-encoded key, yours forever), `publish` a static site to it,
`export`/`import` collections as **`.snk` files (Willow drops)**,
`block` unwanted domains, `remove` without blocking, `serve` the
collection to an ordinary browser at `sneakerweb.localhost`. Sites merge
and forgery is prevented by Willow/Meadowcap; a full protocol
specification exists beyond the CLI. In design: **sneakerdep**
(dependency lists for transitive collections) and collection roots/GC.
**Maturity:** **Released** 2026-W27; **1.2.0** 2026-W40 (base32 domains
fixing Chromium/WebKit hostname limits, `remove`, custom collections);
axum-based speed/big-sites release 2026-W47 (seekable video from the
local store). Real-world distribution demonstrated at DWeb Camp (sites
reached 9 countries by USB key); HN/Lobsters coverage; outside
contributors (Emmy, slug, interfect); workshops running (sneakmeet @ v2
Rotterdam, 13 Dec 2026). Note on names: there is **no separate product
called "sneaker world"** — Sammy uses "sneaker-world" once as a jocular
dateline ("This week's news from sneaker-world"); the project is
sneakerweb, its sites are sneakerwebsites, its events sneakmeets.
**Depends on it:** — (it is the flagship application; it depends on
Willow, Meadowcap, Drop Format, Bab, willow25, ufotofu, Fjall store).
**Relevance:** The existence proof that the whole stack composes into a
shipped product people enjoy using — and the dogfood that drove the Drop
Format streaming revision, the base32 fix, and the store APIs.

## 7. Pillow (with p2panda)
**What:** A joint p2panda × worm-blossom grant project: **Willow "support
peers"** — highly available peers that store/retrieve data for p2panda
apps without understanding p2panda data: decentralised capability-powered
permissions (Meadowcap), happy to evict stale data, efficiently syncing
with each other via **WTP**. Deliverables: the WTP implementation, plus
support-node servers with web UIs for configuring capabilities, data
eviction, and storage usage. Support nodes are configurable Willow peers
(WTP over TCP + HTTP admin UI; storage limits; optional support-node
inter-sync).
**Maturity:** Announced 2026-W35; in active build (Sammy on support
nodes/UI, Emmy on Willow'25/WTP implementation) as of the latest updates.
**Depends on it:** p2panda apps are the named first users.
**Relevance:** Directly the estate's question, answered by the field:
p2panda's own availability layer is being built on Willow+Meadowcap.
Watch this project's deliverables as the WTP's first production test.

## 8. Macromania (+ macro packages)
**What:** A TypeScript-embedded, JSX-syntax templating DSL for generating
strings — "especially good at HTML" — used to generate willowprotocol.org
and worm-blossom.org (definitions/tooltips via **defref**, RSS, code
highlighting). Core lives in its own Codeberg org (`macromania`);
packages published on JSR under `@wormblossom`: macromania (v1.0.2),
macromania-cite, -config, -counters, -defref, -fs, -html,
-idempotentify, -katex, -logger, -memoise, -names, -web (v0.4.2),
-webtt, and **wb-megabar** (the cross-site navigation bar); plus
`getting-started-with-macromania` (example repo). The GitHub repo is a
migration stub pointing at Codeberg.
**Maturity:** Core stable since 2025-W42; in daily use for all their
sites; high-level onboarding docs still missing (their own assessment);
Aljoscha calls it the tooling they use most and progress it least
(2026-W43).
**Depends on it:** all worm-blossom websites; WebTT's reference behaviour.
**Relevance:** Low for the auth/transport decision; it explains the spec
sites' unusual quality (cross-domain definition tooltips) and is a real,
if niche, JSR-published ecosystem of 15 packages.

## 9. WebTT (Web Tooltips)
**What:** A small specification for nestable **cross-domain tooltips with
no server-side logic**: include a compliant JS file + CSS, mark elements
with `data-tooltip-anchor`, tooltips load in an iframe on mouseover.
**Maturity:** Spec published 2025-W50; JS implementation on Codeberg;
macromania-webtt package on JSR; powers the Willow spec sites' tooltips
and defref's cross-domain references (2026-W46).
**Relevance:** Minor, but it's the mechanism that makes their multi-site
spec corpus feel like one hyperlinked document.

## 10. Frugal Async Rust / `frugal_async`
**What:** A set of simplifying assumptions for async Rust (documented on
the Frugal Async Rust page) + a utility crate (e.g. an awaitable Mutex;
smol-based): futures need not be `Send`, runtimes may be single-threaded,
APIs get simpler and cheaper.
**Maturity:** Page + crate released 2025-W42; the doctrine is load-bearing
across ufotofu/willow25.
**Relevance:** The single most consequential *constraint* in their stack
for an adopter: everything is `current_thread`-tokio only (Rocket
incompatibility, 2026-W42; `Send`-bound friction with axum, 2026-W45).

## 11. `compact_u64`
**What:** A Rust crate implementing the machine-friendly variable-length
u64 encoding used throughout the Willow specs — length-up-front (contrast
LEB128), word-at-a-time decoding, lexicographic order = numeric order.
**Maturity:** Released 2025-W47; small and stable by design.
**Relevance:** A borrowable micro-design: order-preserving varints are
directly useful in kv/SQLite index keys.

## 12. `order_theory`
**What:** Rust traits for order theory — successors, min/max, lattices —
born from Willow's grouping needs; generic array implementations reused
by willow25; its property-testing utilities found errors in their earlier
groupings code.
**Maturity:** Published 2025-W45 ("accidentally" — released while tidying).
**Relevance:** The semilattice formalisation under Areas/RBSR, as a
standalone borrowable crate.

## 13. Seasonal Clock (seasonalclock.org)
**What:** Cinnamon's "global clock with non-numerical hours": the day is
divided into named, symbol-bearing fractions (seed, leaf, tree, shell…)
instead of numbers — designed to gently replace Swatch Internet Time.
Stewarded by worm-blossom (repo in the org; the site is theirs to keep).
**Maturity:** Long-standing small app; **Gergely Polonkai's Pebble
e-paper watch port** (via SSB) in progress, with the Hummingcrow 24-icon
set commissioned and delivered (2026-W19, W30).
**Relevance:** None for the protocol decision; included for completeness —
and because the collective's name is one of its hours ("speak to you at
worm-blossom").

## 14. Earthstar (predecessor; Sammy is steward)
**What:** Cinnamon's earlier p2p storage protocol/project (TypeScript),
from which Willow descends: Earthstar dropped SSB-style immutability
(deletion/editing, subset download in any order, one keypair = a person
across devices) but lacked efficient sync for unlinked sets — the gap
Willow's set reconciliation fills. Sammy inherited stewardship in 2022
("mortality driven development": Cinnamon left extensive design docs,
comments, tests). Earthstar 11 is being built on Willow (per the earlier
research note).
**Maturity:** Legacy/predecessor; its community produced Willow's first
external implementer ecosystem (Ourcana's author is an Earthstar lurker-
turned-implementer).
**Relevance:** The lineage explains Willow's non-negotiables: real
deletion, subset sync, person-keypairs.

## 15. wb-box + the playlists (music tooling)
**What:** Their fork of BeepBox (via AbyssBox) — a piano-roll chiptune
player/editor whose distinguishing feature is **playlist pages**; powers
worm-blossom.org's weekly tracks, the Weekly Tracks / Assorted Music
playlists, and Aljoscha's albums (*Computer Has No Feelings*,
*Circumference* — distributed via sneakerweb first).
**Maturity:** In weekly production use since 2025-W49.
**Relevance:** None for the decision; completes the project census (and
explains the "Edit link" saga in the diary).

## 16. Interactive Willow Data Model (toy)
**What:** A browser toy on willowprotocol.org: view/add entries and "sync"
between three peers, to build intuition for the data model.
**Maturity:** Released 2025-W47; Codeberg repo (org listing: "An
interactive toy version of the Willow Data Model," updated 2025-11-21).
**Relevance:** The fastest way to develop a feel for entries/Areas before
reading the specs.

## 17. Willow test vectors (repo)
**What:** Inputs + expected outputs for Willow encodings and data-model
functions (overwrite determination, path successor, capability
verification), maintained on Codeberg; refreshed 2026-W36 after some
vectors had become "outdated and thus actively misleading."
**Maturity:** Actively maintained; used by Freckles (willow-ts) and Heni
(Ourcana) as correctness criteria.
**Relevance:** The interop backbone — and demonstrably a bug-finding
instrument for the reference implementation itself (2026-W37).

## 18. willow-tutorials (repo)
**What:** A Codeberg workspace holding the runnable code for
willowprotocol.org's Rust tutorials (data model, groupings, Store), with
a rustdoc-JSON → site-macro pipeline keeping docs and code in sync.
**Maturity:** Created ~2026-W40; Store tutorial published 2026-W41.
**Relevance:** Onboarding path for any estate developer evaluating
willow25 hands-on.

## 19. Rummager (proposal only)
**What:** A proposed general-purpose GUI application for "organically
expressing your own applications through the act of nesting free-form
data" on Willow.
**Maturity:** Grant proposal **rejected** (2025-W46); prototype only as
spare time allows; no code known.
**Relevance:** None directly; shows their application ambitions beyond
publishing (general personal data).

## 20. Cobweb + Sufficiently Good Media Formats + Cardboard (designs)
**What:** Three design-stage projects from the editorials: **Cobweb**
(2025-W51) — a hypertext system on Willow (transclusion, Willow URIs,
one simple image format, Bab-verified progressive media); **Sufficiently
Good Media Formats** (2026-W01, W38, W39) — unified image/audio/video
formats, quantisation-only lossy step, LZ + predictor residuals, novel
segment-seeking schemes (backwards skiplist; self-synchronising codes),
target ~400 lines of C each; **Cardboard** (Aljoscha's older GitHub spec)
— papers as static websites bundled with transitive citations, the idea
sneakerdep now revives inside sneakerweb.
**Maturity:** Designs/editorials only; no implementations announced.
**Relevance:** Low; the media-format seeking work shows the same
data-structure depth they bring to storage, and Cobweb is the long-term
"what Willow apps could feel like" sketch.

## Independent implementations (not worm-blossom projects — adoption evidence)

- **Ourcana (Dart/Flutter)** — Sarah Aronson's plural-community app
  (~9,000–10,000 users) with **her own Willow implementation in Dart**,
  bit-compatible "over the wire" with the Rust implementation; a WTP-draft
  client; Rust willow25 runs on always-on server nodes; Dart submodule
  planned for open-sourcing. Interview: 2026-W23. Her feedback changed the
  Drop Format spec (streaming/dripdrop).
- **willow-ts (TypeScript)** — Freckles' Willow'25 implementation for the
  browser; "not quite production ready" (2026-W38); its auditing of the
  test vectors found a serious capability-verification bug (2026-W37).
- **Bab in Zig** — by Spritely-adjacent implementers; found the WILLIAM3
  digest bugs (2026-W26) that led to the verified 0.8.0.

---

# Part 3 — What the catalogue adds up to

## The posts that matter most for the estate's decision

1. **2026-W35 (Pillow)** — p2panda itself is building its availability
   layer on Willow peers with Meadowcap permissions, and funding WTP.
   The "p2panda + capability layer" composition the investigation
   converged on is the one the two projects chose themselves.
2. **2025-W52 (maximalism post-mortem + 2026 roadmap)** — Confidential
   Sync's complexity admitted and demoted; pull-based doctrine; WTP and
   the capability-scoped Willow server named as the future. Read before
   citing Confidential Sync as a reason to adopt anything.
3. **2026-W44 + W32 + W35/W36 (WTP design trail)** — WTP's actual status:
   spec published without a wire encoding; message model still being
   merged/symmetrised; stateful *and* stateless operation; Bab-verified
   resumable transfers; RBSR over AreaSlices.
4. **2026-W45 (Amparore/AreaSlices)** — the 3D-storage objection to
   Willow dissolves into denormalised 1D/2D indexes; a SQL-backed store
   becomes a design exercise, not a research project.
5. **2026-W46 (pruning vs scoping) + W37 (capability-verification bug)**
   — the two hardest honesty tests in the diary: a real semantic tension
   in Meadowcap/Willow (mitigated, not solved) and a serious verifier bug
   (caught by their own test-vector programme, via an external
   implementer).
6. **2026-W26 + W28 (WILLIAM3 bugs → verified 0.8.0)** — Bab's maturity
   in one arc: wrong digests in the reference implementation in June
   2026; cross-implementation verification restoring confidence in July.
7. **2026-W23 (Ourcana interview)** — the only production deployment
   story: ~10k users, an independent implementation, and the
   "Rust is a service, Dart/Flutter are for clients" topology — which
   maps neatly onto support-peer thinking.
8. **2026-W33 + W42 (generics stripped; Rocket failure)** — the
   implementation's real shape: concrete, single-threaded, frugal-async.
   Any composition must be designed around `current_thread` tokio and
   no-`Send` futures.
9. **2025-W47 (semilattice editorial) + 2025-W46 ("fancy CRDTs")** — the
   theoretical core: Areas as sublattices unifying access control and
   partial sync; LWW-over-history as a deliberate, argued choice.
10. **2026-W36 (support peers essay + todo list)** — the fungibility×
    uptime framing for relay-like peers, and the author's own list of
    what's *not* done (Willow'25 at-rest encryption instantiation,
    RBSR store traits, ed25519 PQ exposure).

## Surprises

- **The diary is an unusually candid engineering record.** Failed
  experiments (Rocket), broken optimisations (Bab k-grouping), wrong
  digests (WILLIAM3), a serious capability-verification bug, an
  over-complex flagship protocol (Confidential Sync) — all published by
  the authors themselves, usually within a week. For due-diligence
  purposes this blog is worth more than the specs' status labels.
- **"Final" doesn't mean final.** The Data Model and Meadowcap were
  promoted to Final on implementation alone; the collective changed the
  rule in Nov 2026 to require real-world usage — retroactively
  downgrading what their own Final labels certify.
- **Encryption is the thinnest layer.** Drop encryption was *removed*
  from the Drop Format spec (left to apps); Willow'25 at-rest encryption
  has "generic designs, but no specific instantiation yet" (2026-W36);
  encrypted sneakerweb sites are a hope, not a feature. The Encrypted
  Willow analysis in the earlier note is consistent with everything the
  diary says across a full year.
- **The ecosystem is real but small and personal.** Three members, a
  handful of recurring contributors, one production app (Ourcana), two
  in-progress independent implementations (Dart shipped, TypeScript and
  Zig in flight) — and a weekly call that includes p2panda's people.
  Bus factor remains the honest summary: two founders + one new member,
  grant-to-grant funding (Bab's grant ended mid-2026; Pillow is the
  current vehicle).
- **They refuse LLM-generated contributions** (policy formalised
  2026-W46) — relevant if the estate ever wants to upstream patches:
  contributions must be human-written.
- **"Sneaker world" is not a thing** — just a joke dateline. Sneakerweb,
  sneakerwebsites, sneakmeets, `.snk` files, and (in design) sneakerdep
  are the real vocabulary.
- **GPUI:** the diary contains no worm-blossom position on GPUI — only
  Sammy sharing "A Critique of GPUI" (blog.zortax.de) in 2026-W47 with a
  brief personal comment about Elm-style architectures. (Answered here
  because the catalogue is the natural place a reader would look.)

## Sources

- https://worm-blossom.org/archive (index) · /archive_2025_2 ·
  /archive_2026_1 · /archive_2026_2 · front page (W40–W47 duplicates) ·
  /archive/comics/ · /playlists/ (+ /playlists/wb, /playlists/assorted_music)
- https://willowprotocol.org/ · https://bab-hash.org/ ·
  https://sneakerweb.org/ · https://worm-blossom.org/ufotofu/
- https://codeberg.org/worm-blossom (org page snapshot, crawled
  2026-07-06: 14 repos listed with descriptions — sneakerweb site,
  willow_rs, worm-blossom.org source, bab-hash.org site, bab_rs, ufotofu,
  one further repo updated 2026-05-28, compact_u64, frugal_async, WebTT,
  interactive Data Model toy, macromania-fs, getting-started-with-
  macromania, Seasonal Clock; later repos attested from the diary:
  sneakerweb, willow test vectors, willow-tutorials, wb-box,
  willowprotocol.org source) · https://jsr.io/@wormblossom (15 packages)
  · docs.rs (ufotofu).
- Caveat: the live Codeberg org listing could not be re-extracted in full
  on 2026-11-23 (the fetch returned the July snapshot without repo names);
  repo names above are corroborated from the diary, JSR, and search
  results rather than from a fresh org-page read.
