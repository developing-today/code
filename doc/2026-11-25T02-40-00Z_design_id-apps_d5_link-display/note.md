# D5: the link display system, slice 1 (view model and interaction contract)

Design record for #13888 (parent epic #13880, programme order 5).
Inputs: the D3 graph layer record
([`2026-11-24_design_id-graph-layer-typed-links-verbs/D3-graph-layer.md`](../2026-11-24_design_id-graph-layer-typed-links-verbs/D3-graph-layer.md)),
D1 slice 1, the `id://` v2 grammar
([`2026-11-25T01-20-40Z_design_id-apps_d1_id-uri-scheme/`](../2026-11-25T01-20-40Z_design_id-apps_d1_id-uri-scheme/)),
and D2 slice 1, embeds and render states
([`2026-11-25T01-40-00Z_design_id-apps_d2_transclusion/`](../2026-11-25T01-40-00Z_design_id-apps_d2_transclusion/)).
Prior art named by the issue: #9126 (graph query and visualisation research),
#10235 (the bounded, server-paged graph viewer), #13623 (Kitty graphics stills).

Status: slice 1 of 4, with slice 2 in [`slice-2-web.md`](slice-2-web.md), slice 3 in
[`slice-3-kitty.md`](slice-3-kitty.md) and slice 4 in
[`slice-4-conformance.md`](slice-4-conformance.md). The view model and the interaction contract are designed
here and checked by a stdlib reference. Slice 2 (web and WASM renderer), slice 3
(Kitty renderer) and slice 4 (fallback and conformance checklist) build on the
frame defined here; §6 already fixes the list/tree floor they are tested against.

Executable reference: `fixtures/view_reference.py`
(`python3 -I fixtures/view_reference.py check`, stdlib only). It builds a frame
for eight cases over two fixture graphs, checks the invariants in §7, and
compares every frame with `fixtures/expected.json`.

## 1. Problem

Drew asked to "view all the connections between notes in a fancy graph view,
similar to how Obsidian notes look", in the web, in WASM, and in terminals with
mouse hover and click. Three surfaces drawn by three independent views would
disagree within a month about which nodes appear, what a click does and what a
viewer may see. This slice states the one thing all three share: what a view
shows for a question, and what each gesture means.

## 2. Decisions

1. **The view model answers one question: a focus query.** "Show me what is
   connected to this object." Its answer is a **frame**: a bounded set of
   nodes and edges, each with a place in a shared layout. Renderers draw
   frames; they never decide membership.
2. **The view model owns the layout** (the first open question in the issue).
   It places every node in polar coordinates: ring = hops from the focus,
   angle = an integer in 1/65536 of a turn. Renderers only project rings and
   angles to pixels or cells. That is the only way the terminal and the web
   can show the _same_ picture, and it makes the layout testable. A renderer
   may animate between frames, but the resting picture is the model's.
3. **v1 is neighbourhood-only** (the second open question). A frame never
   holds more than its node budget (default 200, hard cap 2000) or reaches
   past its radius (default 2, cap 4). Whole-graph display changes what has
   to be synced underneath (D3, D4), so it waits for Drew (§9, question 1).
4. **Nodes are D3 objects; edges are D3 links.** A link that is itself linked
   to (a reply to a mention, say) also appears as a small **link node**, so
   discussion about a link is reachable from the graph (§3.2).
5. **One edit history, one node.** A supersede chain is one node that shows
   its head and carries an `edited` badge. Supersede links are history, not
   edges (§3.3).
6. **What a viewer cannot read is absent, not hidden.** Unreadable objects
   and links are left out and never counted, as in D2 §5.

## 3. The view model

### 3.1 The query

| Field     | Default            | Meaning                                                                                                    |
| --------- | ------------------ | ---------------------------------------------------------------------------------------------------------- |
| `focus`   | required           | The object at the centre. Any member of a supersede chain focuses that chain's node.                       |
| `radius`  | 2                  | Hops from the focus. 0 to 4.                                                                               |
| `budget`  | 200                | Most nodes in the frame. 1 to 2000.                                                                        |
| `verbs`   | all drawable verbs | Which link verbs are edges and are followed: `reply`, `quote`, `embed`, `mention`, `attach`, `nest`.       |
| `authors` | everyone           | Keep only nodes whose shown object is by one of these authors.                                             |
| `since`   | none               | Keep only nodes whose shown object's `created_claim` is at or after this time.                             |
| `until`   | none               | Keep only nodes whose claim is before this time.                                                           |
| `within`  | none               | Keep only the container and the objects nested under it, transitively (`nest` links): the subgraph filter. |

The focus always passes the filters. A focus the viewer cannot read and one that
does not exist are refused alike (`focus_unavailable`), so a refusal tells a
viewer nothing about objects they cannot see. Other refusals:
`radius_out_of_range`, `budget_out_of_range`, `verb_not_drawable` (asking for
`supersede`), `within_unavailable`, `focus_outside_within`.

`created_claim` is signer-claimed (D3 §2.1). Time filters are allowed because
people think in time, but a frame built with one says `claims_untrusted: true`
and every renderer shows that the window is by claimed time.

`within` is the D1 subgraph filter in structural form. D1 slice 2 owns the
collection URI spelling (`set/`); when it lands, a collection URI resolves to
exactly this node set, and the query takes the URI in place of the container id.

### 3.2 Nodes and edges

A node is one D3 object as one viewer sees it:

| Field           | Meaning                                                                                       |
| --------------- | --------------------------------------------------------------------------------------------- |
| `id`            | The node identity: the object, or the root of its supersede chain.                            |
| `shows`         | The object displayed: the chain head the viewer can read.                                     |
| `kind`, `label` | From the shown object. `label` is the title, or the start of the body, at most 60 characters. |
| `author`        | The shown object's author.                                                                    |
| `hop`, `ring`   | Hops from the focus.                                                                          |
| `angle`         | Place on the ring, 0 to 65535 (§4).                                                           |
| `role`, `dim`   | `focus` (hop 0), `neighbour` (hop 1), `context` (deeper, dimmed). Focus plus context.         |
| `parent`, `via` | The node and link it was reached through: the layout's spanning tree.                         |
| `badges`        | `edited` (the chain has more than one version), `fork` (more than one head).                  |
| `more`          | Readable neighbours, through edges the query allows, that the frame left out.                 |

An edge is one D3 link between two nodes in the frame: `id` (the link's
`object_id`), `verb`, `source`, `target` (D3's direction: the source speaks
about the target), `author`, a semantic `style` (§3.5) and `dim`. Every link
between two nodes in the frame is drawn, not only the spanning-tree links; the
list/tree floor lists the others as "also linked".

**Link nodes.** D3 makes every link an object, so a link can be the target of
another link: someone replies to a mention, or quotes a citation. When a drawable
link ends at a link object, that link object becomes a node of kind `link`,
joined to its own source and target by two structural `endpoint` edges. The
endpoint edges are not D3 links; they exist so the discussion about a link is one
hop from the link. They are drawn only when the linked link's verb is allowed.

### 3.3 Supersede, forks and invalid supersedes

Every member of a supersede chain maps to one node, keyed by the chain's root.
The node shows the head. Edges to and from any version attach to the node, so an
old reply to version 1 and a new mention of version 3 meet at the same place.
This is D3's rule that only the chain head renders in normal views; history is
the object page's job, reached by opening the node.

- A chain with two readable heads is a **fork**: the node shows the head with the
  lowest object id and carries `fork`; opening it names the chain head form, and
  D1 already says that resolving a forked head returns every head.
- A supersede whose author differs from its target's author is not valid without
  a moderation grant (D3 §5). The view ignores it, lists it in
  `ignored_supersedes`, and the would-be replacement stays its own node. The
  fixture `l-hijack` shows this.

### 3.4 Expansion order

The frame grows breadth first from the focus. From each node, its edges are
taken in a fixed order: verb rank (`reply`, `quote`, `embed`, `mention`,
`attach`, `nest`, then `endpoint`), then inbound before outbound, then the
neighbour's claimed time, then its id. Replies first is the issue's canonical
Zettelkasten query ("everything connected to this note, replies first"), and
inbound-first puts what others said about the focus ahead of what it points to.
When the budget runs out the frame stops adding nodes and sets `cuts.budget`;
the nodes left out are counted in their neighbours' `more`. A node at the
radius is not expanded, and `cuts.radius` says the radius cut something.

Input order never changes a frame: the reference rebuilds every case from the
fixture with every list reversed and requires byte-identical frames.

### 3.5 Edge styles

Styles are semantic tokens. Each renderer maps them to its own lines, colours
or glyphs, keeping them distinguishable. They come from D3's per-verb rendering
hints:

| Verb       | Style              | Reading                                                        |
| ---------- | ------------------ | -------------------------------------------------------------- |
| `reply`    | `conversation`     | Part of a thread.                                              |
| `quote`    | `pinned-reference` | Refers to fixed bytes; may show a freshness badge on identify. |
| `embed`    | `transclusion`     | Shows the target inside the source (D2).                       |
| `mention`  | `reference`        | Names the target.                                              |
| `attach`   | `carry`            | Carries the target as part of the source.                      |
| `nest`     | `containment`      | Placed inside the target.                                      |
| `endpoint` | `link-endpoint`    | Joins a link node to its ends (not a D3 link).                 |

Unknown verbs follow D3's reader rule: the link is kept and skipped. It is never
drawn and never followed, the endpoints still appear through other links, and
the frame counts what it skipped in `skipped_unknown_verbs`.

## 4. Layout

The layout is a radial tree over the frame's spanning tree (each node's
`parent`). The focus sits at the centre (ring 0, angle 0). Ring _n_ holds the
nodes _n_ hops out. Every node owns a sector of the turn: the focus owns all of
it, and each child gets a share of its parent's sector in proportion to the
number of leaves under it, in expansion order. A node sits at the middle of its
sector. Arithmetic is exact (rationals), and the angle is floored to an integer
in 1/65536 of a turn, so every implementation in every language computes the
same number.

Because every leaf gets at least 1/2000 of a turn (about 32 units), no two
nodes on one ring share an angle; the reference checks this on every frame.

What this layout chooses, deliberately:

- **Hops are distance.** In a Zettelkasten view, "how far is this from my note"
  is the first thing people read. A force layout hides it.
- **The picture is stable under small changes.** A new reply adds a leaf inside
  its parent's sector; the other sectors shrink in proportion but keep their
  order.
- **It fits a terminal.** Rings and angles map to cell grids as well as to
  pixels; slice 3 decides the aspect correction.

Slice 2 may let a renderer relax positions for legibility (overlap removal) as
long as it keeps ring order and angular order; the conformance checklist in
slice 4 will test exactly those two orders.

## 5. The interaction contract

Five intents. A renderer maps its own gestures onto them (slice 2 for pointer
and touch, slice 3 for terminal mouse reporting), and every intent must also be
reachable from the keyboard, which is how the list/tree floor satisfies the same
contract.

| Intent          | Target | Answer                                                                                                                                                                | Frame changes |
| --------------- | ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------- |
| `identify`      | node   | Show label, kind, author, claimed time (marked as claimed), badges and a preview of the shown object. Highlight the node's edges and adjacent nodes.                  | no            |
| `identify-link` | edge   | Show the verb, the link's author and its two ends. Highlight the edge and both ends.                                                                                  | no            |
| `open`          | node   | Navigate to the object's page by its D1 URI: `obj/` with the shown object, or `head/` with the chain root when the node is `edited`, so the page follows later edits. | no            |
| `inspect-link`  | edge   | Navigate to the link object's own page (`obj/` with the link id). A link is an object and has a page.                                                                 | no            |
| `focus`         | node   | Build a new frame with the same query and this node as the focus.                                                                                                     | yes           |

The issue's shorthand maps onto these: hover is `identify` (or
`identify-link`), click is `open`, click on an edge is `inspect-link`. Focus
plus context is `focus`: the chosen node moves to the centre, its neighbourhood
becomes bright and the rest becomes dimmed context. Slice 2 names the gesture
for `focus` (for example a modifier click or a double click); it must not be
the plain click, which belongs to `open`.

Rules every renderer keeps:

- An intent may only target what the current frame holds; anything else is
  refused (`target_not_in_frame`). A renderer never fetches objects the frame
  does not name, so it cannot show more than the model let the viewer see.
- `identify`, `identify-link`, `open` and `inspect-link` never change the frame.
- Claimed times are always presented as claims.
- The display form of a URI (what a person sees for `id://…`) is D1 slice 5's.
  Until it lands, renderers show the label and keep the URI behind `open`.

## 6. The list/tree floor

Every frame renders as plain lines. This is the accessible view, the scripting
view, the fallback when a terminal has no graphics or no mouse, and the floor of
the slice 4 conformance checklist. For `notes-replies-first` it reads:

```text
Spaced repetition [note]
  <- reply: Re: spacing works for vocabulary [note]
    <- reply: Re: and for formulas? [note]
  <- quote: Forgetting curve (Ebbinghaus, 1885) [note] (edited)
    <- mention: Sleep consolidates memory [note]
    <- endpoint: mention link [link] (+1 more)
  <- mention: Index: memory [note]
  -> mention: Leitner boxes [note]
    -> attach: Photo of the box shelf [image]
also linked:
  mention link -endpoint-> Sleep consolidates memory
  Spaced repetition -mention-> Forgetting curve (Ebbinghaus, 1885)
```

The rules: the spanning tree depth first in angle order, two spaces per hop,
`<-` when the child is the link's source (it speaks about its parent), `->`
when it is the target, the verb, the label, the kind, badges in parentheses,
`(+N more)` when the frame left neighbours out, then every non-tree edge under
"also linked", then "(node budget reached)" when the budget cut. The line order
is the angular order of the graphic renderers, so the floor and the picture
agree on what comes first. Every intent works on a line: a selected line is a
target.

## 7. Fixtures and the reference

`fixtures/graph.json` holds the two use cases the issue validates against:

- **The Zettelkasten notes graph**: notes with replies, mentions, a quote, an
  attached photo, an edited note (a supersede chain), a reply to a mention link,
  a note in a private area only `bo` can read, an unknown verb (`cites-v9`), and
  a supersede by the wrong author.
- **The ID programme**: the #13880 epic with its designs and research nested in
  it, their dependency mentions, D5 embedding D2, an edited D1, the core graph
  app (#14037), a triage comment, and a design outside the epic.

The eight cases in `CASES` cover: replies first; the owner of a private note
seeing it while others see neither it nor its count; a budget cut with `more`
counts; an edited node and a link node; a time window (claims untrusted); the
epic as a containment view; the `within` subgraph filter; and an author filter.

`check` asserts, for every frame: no duplicate nodes, the budget and radius
held, one focus at the centre, every node and edge readable by the viewer, no
two nodes sharing an angle on a ring, no dangling or filtered-out edges, and the
same frame for reversed input. It also checks the contract: `focus` on every node
of a frame gives a valid frame centred there, `open` and `identify` never change
the frame, `identify` presents claims as claims, `inspect-link` opens the link
object, `open` on an edited node names the chain head form, and each refusal code
is produced where it should be.

## 8. Alternatives

| Alternative                                                  | Verdict  | Why                                                                                                                                                                            |
| ------------------------------------------------------------ | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Each renderer lays out its own picture (force layout in web) | Rejected | The terminal and the web would never show the same picture, and nothing about the layout could be tested. Force relaxation survives only as a renderer's legibility pass (§4). |
| Force-directed layout in the model                           | Rejected | Not stable under small changes, not exact across languages, and it hides hop distance. A deterministic force layout is possible but buys nothing the radial tree does not.     |
| Server-rendered images for terminals (the #10235 shape)      | Rejected | No hover or click, and stale as soon as the graph moves. #13623's stills pipeline stays useful for previews inside `identify`.                                                 |
| Adopting a web graph library's data model                    | Rejected | Untyped edges and no link objects fight D3. Slice 2 may still borrow a library's drawing code behind this model.                                                               |
| Hiding unreadable nodes as anonymous "locked" nodes          | Rejected | It reveals that something exists and how many, which D2 §5 already refuses for embeds.                                                                                         |
| Drawing every supersede version as its own node              | Rejected | The graph would fill with old versions; D3 says only the head renders. History stays one `open` away.                                                                          |

## 9. Open questions

1. **Whole-graph display (Drew decides).** v1 is neighbourhood-only. A
   whole-graph view of thousands of notes needs the whole graph on the device,
   which changes D3's partial replication and D4's bundles. Until Drew asks for
   it, the budget cap stays at 2000.
2. **Gesture for `focus` (slice 2 decides).** Plain click is `open`; slice 2
   picks the focus gesture for pointer and touch and slice 3 for the terminal.
3. **Link node labels (slice 2 decides).** The reference labels a link node by
   its verb ("mention link"). A richer label, such as both ends, may not fit a
   terminal cell row; slice 2 and 3 settle it with the D1 display form.

## 10. What slices 2 to 4 inherit

- The frame (§3.2) and the query (§3.1) as the only input a renderer reads.
- Polar layout coordinates and the two orders a renderer must keep (§4).
- The five intents and their rules (§5).
- The list/tree floor and its exact line format (§6).
- The fixtures and `view_reference.py` as the oracle a renderer's frame
  builder is tested against.
