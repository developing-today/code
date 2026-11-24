# Worm Blossom projects addendum — Rummager, Cobweb, Macromania, Willow URIs, WebTT

Lane WB-PROJECTS for the auth + transport investigation
([developing-today/code#13779](https://github.com/developing-today/code/issues/13779)).
Researched 2026-11-23. Baseline: the catalogue entries in
[worm-blossom-catalogue.md](worm-blossom-catalogue.md) (§8 Macromania, §9 WebTT,
§19 Rummager, §20 Cobweb, and the diary entries cited per project), extended
from primary sources: the [projects page](https://worm-blossom.org/projects/),
the [WebTT page](https://worm-blossom.org/webtt), the
[Willow URIs specification](https://willowprotocol.org/specs/uris/)
(read from the local mirror at `~/workspace/worm-blossom-mirror/willowprotocol.org/specs/uris/`),
[JSR package pages](https://jsr.io/@wormblossom/macromania), and the diary
archive / [TODOs 2025 page](https://worm-blossom.org/notes/todos2025).
One sourcing caveat: Codeberg returned HTTP 429 throughout this pass, so
repo-level details rest on JSR, the project pages, and the diary rather than
fresh repo reads; the catalogue's earlier Codeberg org listing corroborates
the repo layout.

State labels up front: **Macromania — released, in daily production use
(theirs). WebTT — released (spec + implementation, in production on their
sites). Willow URIs — spec published, Proposal status, no implementation
found. Rummager — proposal only, grant rejected, no code. Cobweb — design
sketch only, no implementation.**

---

## 1. Rummager — proposal only

**What it is.** A proposed general-purpose GUI application on top of Willow.
Sammy's grant pitch (diary, 2025-W45) described it as a way of "organically
expressing your own applications through the act of nesting free-form data"
([catalogue §19](worm-blossom-catalogue.md); [diary archive](https://worm-blossom.org/archive_2025_2)).
The collective's own TODO page states the idea more concretely: "a
Willow-powered, collaborative, multi-media data publishing platform allowing
for arbitrary nesting as the only organisational principle" — with the
ambition to "move from Discord to Rummager for Willow/worm-blossom community
interaction" ([TODOs 2025](https://worm-blossom.org/notes/todos2025)). The
current projects page describes it in the same terms: "a collaborative tool
for manipulating nested data, flexible enough to be useful for a variety of
different tasks (chat, forums, image sharing, etc)", illustrated with
hand-drawn UI sketches ([projects page](https://worm-blossom.org/projects/)).

**How it would work.** The single organisational primitive is _nesting_:
Willow entries arranged in nested paths, multi-writer and capability-scoped
via Meadowcap, and the "applications" (a chat, a forum, an image board) are
not separate programs but emergent shapes of how a group nests and arranges
its data. It is the personal/small-group data environment the collective
imagines on top of the store — the application-layer counterpart to
sneakerweb's publishing. Aljoscha's TODOs also note an open design question:
assessing how much his bulk-transclusion ideas (see Cobweb) impact Rummager
([TODOs 2025](https://worm-blossom.org/notes/todos2025)).

**State.** Proposal only, and stalled. The grant proposal was **rejected in
2025-W46**; Sammy's response was "we'll just need to build out a prototype in
our spare time" ([catalogue, 2025-W46 entry](worm-blossom-catalogue.md)).
No repository or prototype has surfaced since; the projects page still
frames it as "an application we would like to build." With the team's funded
time going to Pillow/WTP through 2026, Rummager is effectively dormant.

**Relation to the stack.** Pure application layer: it consumes Willow +
Meadowcap and adds nothing protocol-side. It exists to answer "what is a
general-purpose Willow app?" — and currently answers it only on paper.

**Estate relevance.** None directly. Its value to us is diagnostic: the
Willow application ecosystem is so young that even the protocol's own
authors have not shipped a general Willow application — the one they most
want to build is an unfunded sketch. Anyone adopting Willow today is
adopting protocols, not applications.

## 2. Cobweb — design sketch only

**What it is.** A hypertext system on Willow, floated in Aljoscha's
2025-W51 diary editorial and still listed on the projects page as "an
application we would like to build on top of Willow: an efficient hypertext
system exploring some ideas around bulk transclusion"
([projects page](https://worm-blossom.org/projects/); [catalogue §20](worm-blossom-catalogue.md)).
Think Gopher/Gemini-grade simplicity, but where documents live in a
multi-writer, capability-scoped Willow store instead of on servers — the
named inspirations are Gopher, Gemini, man pages, RFCs, Mastodon,
TiddlyWiki, and Joplin (catalogue, 2025-W51 entry).

**How it would work.** Four design commitments from the editorial:
(1) **bulk transclusion** as a built-in — embedding many documents at once
("directory-scale iframes"), so a page can aggregate whole collections
rather than link-or-embed one target at a time; (2) **Willow URIs as the
link format**, which makes links point at mutable entries — content can be
updated without link rot, the deliberate opposite of content-addressed
immutable linking; (3) **extreme authoring simplicity** — a markdown-subset
markup and exactly one simple image format; (4) **efficiency** — a binary,
mmap-able storage format, with media interleaved so images render
progressively and every prefix of the stream is Bab-verifiable as it
arrives. The "Sufficiently Good Media Formats" design work (unified simple
image/audio/video formats, 2026-W01 and W38–39) is largely Cobweb-driven:
one format per media type is a Cobweb requirement, not an independent
whim. An older Aljoscha spec, **Cardboard** (papers as static websites
bundled with transitive citations), is a recognisable predecessor; part of
it now revives inside sneakerweb's sneakerdep (catalogue §20).

**State.** Design/editorial only. No implementation has been announced; it
sits downstream of two things that don't exist yet either (a Willow URI
implementation and the media formats). It is the long-term "what Willow
apps could feel like" sketch.

**Relation to the stack.** Cobweb is to Willow what a browser is to HTTP:
the intended reading/aggregation surface. Its conservative sibling
**sneakerweb** — static sites over the Drop Format, exchanged on physical
media — is the version of the vision that actually shipped; Cobweb is the
live-sync, transcluding version that hasn't.

**Estate relevance.** Low. Nothing to adopt or interoperate with. It is
worth knowing as the clearest statement of where Willow's designers think
hypertext on mutable, capability-scoped data goes (transclusion +
mutable links), and as another data point that the family's application
layer is vision, not software.

## 3. Macromania — released; the collective's production site generator

**What it is.** A TypeScript-embedded domain-specific language for
generating strings — "especially good at HTML" — using JSX syntax. Its own
JSR page puts it precisely: "an highly expressive but completely
unopinionated templating language" taking inspiration from Lisp-like macro
expansion and TeX, designed for "meaningful error reporting, principled
interaction between stateful macros, and static typing for the input
expressions" — and, pointedly, "not a UI library… It produces static text"
([JSR: @wormblossom/macromania](https://jsr.io/@wormblossom/macromania)).
It generates willowprotocol.org and worm-blossom.org — including the
definition/tooltip system (defref), RSS feeds, and code highlighting that
make their spec sites feel like one cross-linked document
([catalogue §8](worm-blossom-catalogue.md)).

**How it works.** You write JSX-looking markup in TypeScript (Deno-first);
macros are typed functions that expand to text, can carry state, and
compose. Around the core sits a family of **15 packages published on JSR
under `@wormblossom`**: the core (v1.0.2), plus -cite, -config, -counters,
-defref, -fs, -html, -idempotentify, -katex, -logger, -memoise, -names,
-web (v0.4.2), -webtt, and wb-megabar (the cross-site navigation bar), with
a `getting-started-with-macromania` example repo. The core lives in its own
Codeberg org (`macromania`); the GitHub repo is a migration stub pointing
there (catalogue §8). 2026 development concentrated in **macromania-web**
(cross-domain URL references, asset moving/transformation, automatic
stylesheet/script dependency injection, principled KaTeX rendering) and in
porting **defref** to support _cross-domain_ references via WebTT — so
worm-blossom.org can now define-discuss terms (e.g. AreaSlices) whose
definitions live on willowprotocol.org (catalogue, 2026-W42 and W46
entries).

**State.** Released and load-bearing — for them. Core stable since
2025-W42 and in daily use for all their sites; the package family is still
actively publishing (macromania-webtt 0.2.3 shipped days before this
research, November 2026 — see WebTT below). Two honest debits from the
diary itself: Aljoscha calls Macromania "of all our tooling the one we are
currently using the most" yet the one they progress least (2026-W43), and
high-level onboarding documentation is still missing by their own
assessment (catalogue §8). The 2025-W52 maximalism post-mortem also names
the Macromania site rewrite "congealing into a mega-branch" as one of the
collective's formative process failures — the tool is good; the rewrite it
enabled was a lesson.

**Relation to the stack.** Orthogonal to Willow-the-protocol: it is the
_publishing_ layer for the whole project (specs, diary, sites), and the
substrate WebTT's reference behaviour is expressed in (via
macromania-webtt).

**Estate relevance.** Low, and indirect. We will not adopt a niche,
documentation-poor TypeScript templating DSL. But defref + WebTT together
are a genuinely interesting _documentation technique_ — define a term once,
reference it from any of several independently hosted sites, get a tooltip
of the canonical definition — which is the closest existing analogue to
what our own multi-book documentation (fleet/atlas/id/mcpx) might want for
cross-references. Worth remembering as a pattern, not a dependency.

## 4. Willow URIs — spec published (Proposal); no implementation found

**What it is.** The `willow://` URI scheme: a standardised, human-readable
way to _name_ Willow resources — the stack's missing URL. The specification
opens: "each willow:// URI identifies either an Entry (optionally together
with a single contiguous slice of its Payload), or an AreaOfInterest in
some namespace" ([Willow URIs spec](https://willowprotocol.org/specs/uris/)).
Spec status, per its own header: **Proposal**.

**How it works.** The syntax mirrors the data model:
`willow://namespace.subspace/path/components?select_features#fragment` for
entries (example from the spec: `willow://family.alfie/blog/idea.txt?from=5&digest=b287af#about`),
and the same shape with `?area&count=12`-style parameters for areas. Four
semantic details reward an engineer's attention:

- **Digest pinning that respects deletion.** An entry URI may carry an
  expected payload digest; it then identifies only an entry with exactly
  that payload. The spec is explicit that this does _not_ pin data against
  pruning: "using an Entry URI will not 'pin' it in any way… retrieval
  deliberately fails if the intended Entry got pruned." Deletion wins over
  links, by design.
- **Payload slices.** Optional start/end indices identify a contiguous
  subslice of the payload, with out-of-range indices truncated — a URI can
  address a byte range of an entry, not just the entry.
- **Area URIs.** An area URI carries the AreaOfInterest's bounds
  (max_count, max_size, time range) — a link can denote "this slice of the
  dataset," the same Area object Meadowcap grants over and sync
  reconciles.
- **Resolution hints.** Every URI contains a (possibly empty, ordered)
  sequence of _other_ URIs serving as hints for how/where to obtain the
  data — the scheme is transport-agnostic and expects resolution to be
  bootstrapped from ordinary web hints.

The spec is carefully RFC 3986-conformant (percent-encoding, dot-segment
normalisation, relative references inherited as URI references) and
parameterised: an instantiation must supply host-safe encodings for
namespace/subspace ids and a query-safe encoding for payload digests
(Willow'25 provides the defaults).

**State.** Spec published **2025-W51** (December 2025) alongside the Drop
Format revision (catalogue, 2025-W51 entry). The 2025-W52 roadmap listed
"Willow URIs implementation" among 2026 work, but as of this research **no
released implementation has been found** — not in willow25, not elsewhere.
It remains a published Proposal awaiting code.

**Relation to the stack.** The naming/addressing layer: Meadowcap says who
may write, WTP moves data, URIs are how anything _refers_ to data. Cobweb
is its flagship intended consumer (mutable links without link rot);
sneakerweb conspicuously does without it (subspace key = domain, served
locally), which shows the scheme is for the live, linked web of Willow
data rather than the sneakernet.

**Estate relevance.** Modest and conceptual. If the estate ever adopts
Willow-family storage, this is the link format that comes with it. As a
design, it is a clean pattern for content references generally —
namespace.subspace/path + optional digest expectation + optional slice +
ordered resolution hints — and its "digest pin fails rather than pins"
choice is a small, sharp statement of the family's deletion-first values
that contrasts with IPFS-style addressing.

## 5. WebTT (Web Tooltips) — released; spec + implementation in production

**What it is.** A small specification for **nestable, cross-domain tooltips
with no server-side logic whatsoever**. The project's own page: "WebTT (Web
Tooltips) are nestable, cross-domain tooltips without any server-side
logic… Tooltips seamlessly work across domains. Tooltips are isolated in
iframes, no need [to] trust tooltip authors. Tooltips can nest arbitrarily
— and even recursively" ([WebTT](https://worm-blossom.org/webtt)). It is
the mechanism behind the Willow spec sites' definition tooltips — hover a
term on willowprotocol.org and you get its canonical definition, wherever
it is hosted.

**How it works.** Three components ([WebTT](https://worm-blossom.org/webtt)):
(1) _tooltip definitions_ — arbitrary self-contained static HTML pages,
served from any domain; (2) _references_ — any HTML element carrying a
`data-tooltip-anchor=<tooltip URL>` attribute; (3) _one client-side
JavaScript file_ (plus CSS) included on both referencing pages and tooltip
pages, which loads the tooltip page in a **sandboxed iframe** on hover.
Nesting falls out naturally: a tooltip page can itself contain anchors.
The specification proper prescribes only the _required communication flow_
between tooltip page and containing page (how heights/presence are
negotiated); how tooltips are displayed, timed, or dismissed is explicitly
implementation-defined, so independent compatible implementations are
possible. The authors' own limitation list is candid: iframe tooltips
cannot be styled by the host page; a tooltip cannot communicate a
preferred width; their current JS sometimes reports an incorrect height.

**State.** Released. Spec published **2025-W50** (December 2025); the
reference JavaScript implementation lives on Codeberg
(`codeberg.org/worm-blossom/webtt`, linked directly from the spec page);
the Macromania integration, `@wormblossom/macromania-webtt`, is on JSR at
**0.2.3, published days before this research** (November 2026) — actively
maintained ([JSR: @wormblossom/macromania-webtt](https://jsr.io/@wormblossom/macromania-webtt)).
In production on the collective's sites since the spec landed, and since
2026-W46 it also carries defref's cross-domain references (catalogue §9
and 2026-W46 entry).

**Relation to the stack.** Not a Willow protocol at all — it belongs to
the Macromania publishing layer — but it is load-bearing for how the
family presents itself: the multi-site spec corpus (willowprotocol.org,
worm-blossom.org, bab-hash.org, sneakerweb.org) reads as one hyperlinked
document because definitions resolve across domains at hover time.

**Estate relevance.** Minor but real, as a technique rather than a
product: WebTT is a working proof that _static, serverless cross-references
between separately hosted documents_ are practical — no API, no build-time
coupling, just a convention plus one script. If our published books ever
want live cross-book definition references, this is the lightest known
pattern for it. As infrastructure for fleet/id/atlas/mcpx or the iroh work,
it is irrelevant.

---

## Bottom line for the five

Two of the five are real, shipped software — and both are _publishing
tooling_ (Macromania, WebTT), not p2p technology. One is a finished-on-paper
naming spec waiting for an implementation (Willow URIs). Two are
application visions with no code at all (Rummager, Cobweb). That
distribution is itself the finding: worm-blossom's protocol layer is
genuinely built, its site tooling is genuinely built, and its application
layer — the layer that would show what Willow is _for_, end-user-side —
remains sketches, one rejected grant, and good intentions.
