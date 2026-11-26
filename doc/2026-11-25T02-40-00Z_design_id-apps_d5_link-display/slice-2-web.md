# D5: the link display system, slice 2 (web and WASM renderer)

Design record for #13888 (parent epic #13880, programme order 5), slice 2 of 4.
It builds on slice 1 in [`note.md`](note.md): the focus query and the frame
(§3), the polar layout (§4), the five intents (§5) and the list/tree floor (§6).
Slice 1 left two questions to this slice: the gesture for `focus` (§9 question 2)
and the label of a link node (§9 question 3). Both are answered here.

Executable reference: `fixtures/web_reference.py`
(`python3 -I fixtures/web_reference.py check`, stdlib only). It projects every
slice 1 frame into three viewports, hit-tests nodes and edges, compares the
keyboard order with the floor, checks refocus identities and the gesture table,
and builds a frame from a 10 000-note graph within the budget cap.
`python3 -I fixtures/web_reference.py scale` times larger graphs.

## 1. Problem

Slice 1 says what a view shows and what each gesture means. A browser still has
to draw it, take pointer, touch and keyboard input, move between frames without
losing the reader, and stay usable at the 2000-node cap. Drew also asked for the
view "in WASM", so the same picture must work where there is no server: a local
store, an offline bundle, or the single-file HTML version that `GET
/v1/apps/{app}/bundle.html` already serves.

## 2. Decisions

1. **One frame builder, two hosts.** The slice 1 view model becomes a Rust crate
   (`id-graph-view`, no I/O) that turns a graph and a query into a frame. The
   server links it behind `GET /v1/graph/frame`; the browser links the same
   crate compiled to WASM for local and offline stores. Both hosts produce
   byte-identical frames for the same graph, viewer and query, checked against
   `fixtures/expected.json`.
2. **Readability is enforced where the data lives.** On the server the frame
   builder runs on the server's store with the session's viewer, so a browser
   never receives an object the viewer cannot read. In WASM the store already
   holds only what the device replicated for that viewer; the builder runs the
   same readability check, so a shared device with two profiles still gets two
   pictures.
3. **Canvas 2D draws; the DOM carries meaning.** The picture is one `<canvas>`.
   Beside it, in the same component, the list/tree floor (slice 1 §6) is a real
   DOM tree (`role="tree"`, one `treeitem` per line) in the same order. Screen
   readers, find-in-page, copy and scripting use the tree; the canvas is
   `aria-hidden`. Selection is shared, so selecting a line highlights the node
   and hovering a node highlights the line.
4. **The renderer only projects.** Rings and angles map to pixels by one
   function (§3). The renderer may nudge labels and, optionally, positions for
   legibility, but only if ring order and clockwise angular order survive; the
   reference checks both on every frame and the slice 4 checklist will too.
5. **`focus` is double click on a pointer, long press on touch, `f` on the
   keyboard.** Plain click stays `open`. This answers slice 1 §9 question 2.
6. **A link node is labelled by its verb and its two ends, shortened.** "mention:
   Spaced repetition → Forgetting curve" fits a tooltip and a tree line; when the
   ends do not fit, the label falls back to slice 1's "mention link". This
   answers §9 question 3 for the web; slice 3 may keep the short form for
   terminals.

## 3. Projection and fit

Angle 0 is 12 o'clock and angles grow clockwise, so the floor's first child is
the first node a reader's eye meets after the focus.

```text
spacing = (min(width, height) / 2 - 24) / max(outer_ring + 0.5, 1) * zoom
theta   = 2π · angle / 65536
x       = cx + ring · spacing · sin θ
y       = cy - ring · spacing · cos θ
```

`cx, cy` is the viewport centre plus the pan offset. At zoom 1 the outermost
ring fits the shorter side with a 24 px margin, which the reference checks at
1280×800, 800×1280 and 360×640 for every case. Zoom runs from 0.25 to 8 around
the pointer (or the pinch centre); pan is a drag on empty canvas or two-finger
scroll. Fit (the `0` key, or a double click on empty canvas) returns to zoom 1
and no pan.

**Level of detail.** Labels are drawn only where they fit:

| Zoom × ring spacing | Shown                                                                 |
| ------------------- | --------------------------------------------------------------------- |
| under 24 px         | Dots and edges only; the focus and the selected node keep labels.     |
| 24 to 64 px         | Labels for the focus, hop 1 and the selected node's neighbours.       |
| over 64 px          | Every label, truncated to the space before the next node on the ring. |

Dimmed context nodes (slice 1 `dim`) draw at 40 % opacity and never take a
label before a bright node on the same ring.

**Optional relaxation.** A renderer may move nodes apart along their ring to
remove label overlap, but never across a neighbour on the same ring and never
off the ring. Relaxation is a legibility pass on screen positions only: the
frame, the floor and the hit-test all keep the model's ring and angle.

## 4. Hit-testing

The target under the pointer is the nearest node within its hit radius (6 px at
zoom 1, growing with zoom), else the nearest edge within 4 px of its line, else
nothing. Nodes win over edges, so a click at a node's centre never inspects an
edge that passes through it. On touch the radii are doubled to meet a 44 px
target. The reference checks that every node is hit at its centre and every
edge at its midpoint whenever no node sits that close.

`(+N more)` draws as a small chip outside the node, on the side away from the
focus. It is a target of its own: activating it is `focus` on that node, which
is how a reader walks past the budget.

## 5. Gestures

| Device   | Gesture                           | Intent          |
| -------- | --------------------------------- | --------------- |
| Pointer  | Hover a node                      | `identify`      |
| Pointer  | Hover an edge                     | `identify-link` |
| Pointer  | Click a node                      | `open`          |
| Pointer  | Click an edge                     | `inspect-link`  |
| Pointer  | Double click a node               | `focus`         |
| Pointer  | Click a `+N more` chip            | `focus`         |
| Touch    | Tap a node                        | `identify`      |
| Touch    | Tap an edge                       | `identify-link` |
| Touch    | Tap Open on the card              | `open`          |
| Touch    | Tap Inspect on the card           | `inspect-link`  |
| Touch    | Long press a node, or Focus       | `focus`         |
| Keyboard | Arrow keys (move the selection)   | `identify`      |
| Keyboard | `l` (cycle the selection's edges) | `identify-link` |
| Keyboard | Enter                             | `open`          |
| Keyboard | Shift+Enter (on an edge)          | `inspect-link`  |
| Keyboard | `f`                               | `focus`         |

Touch has no hover, so a tap identifies and opens a small card with Open,
Focus and, for an edge, Inspect. A plain click opens on a pointer because that
is what people expect of a link. So that a double click does not also open, the
renderer waits the platform double-click interval before acting on a single
click. Middle click and
Ctrl/Cmd click on a node open in a new tab, as any link would.

**Keyboard order is the floor's line order.** Down and Up walk the spanning
tree depth first in angle order, the same order as the tree lines; Left goes to
the parent and Right to the first child. The reference checks that the keyboard
stops and the floor's lines agree one for one. Every intent is reachable from
the keyboard, so the canvas and the tree satisfy the same contract.

## 6. Transitions

A `focus` builds a new frame. Nodes keep their `id` across frames (slice 1 keys
nodes by chain root), so the renderer interpolates each surviving node from its
old ring and angle to its new ones over 250 ms, along the shorter way round the
circle; new nodes fade in at their place and departing nodes fade out. The
reference checks that refocusing on any node of a frame keeps that node. With
`prefers-reduced-motion`, the frame swaps at once and the selected node keeps
its highlight so the reader does not lose their place.

The URL carries the query (`?focus=…&radius=…&verbs=…`), so Back returns to the
previous frame and a frame can be bookmarked or shared; a shared URL still
builds the frame for whoever opens it.

## 7. Scale

The cap is the frame budget, not the graph size. The reference builds frames
from synthetic public graphs:

| Graph                    | Query                 | Frame                    |
| ------------------------ | --------------------- | ------------------------ |
| 10 000 notes, fan-out 4  | radius 4, budget 2000 | 341 nodes, no budget cut |
| 10 000 notes, fan-out 30 | radius 2, budget 2000 | 931 nodes, no budget cut |
| 50 000 notes, fan-out 8  | radius 4, budget 2000 | 2000 nodes, budget cut   |

The stdlib reference takes about 50 ms for the first two and 400 ms for the
third; these are a reference, not a target. The targets for the Rust builder
are 50 ms for a 2000-node frame on the server and 150 ms in WASM on a mid-range
phone, measured in the slice 4 checklist. The canvas draws 2000 nodes and their
edges in one pass per animation frame; hit-testing uses a uniform grid rebuilt
only when the frame, zoom or pan changes. Past the budget, the `(+N more)`
chips and `focus` are how a reader goes further.

## 8. Routes and click-through

Reserved here, built in the implementation slices:

| Route                     | Method | Purpose                                                              |
| ------------------------- | ------ | -------------------------------------------------------------------- |
| `/v1/graph/frame`         | GET    | One frame for the session's viewer; the slice 1 query as parameters. |
| `/v1/graph/frame/respond` | POST   | An intent on a frame; refusals as slice 1 names them.                |
| `/graph`                  | GET    | The graph view page; its query string is the frame query.            |
| `/obj/{id}`, `/head/{id}` | GET    | Object pages for `open` and `inspect-link`, mirroring the D1 forms.  |

The page routes mirror the D1 URI shapes (`obj/` for one object, `head/` for a
chain that follows later edits) so that when D1 slice 5 fixes display forms, the
web pages already sit where the URIs point. Until then the renderer shows labels
and keeps URIs behind `open`, as slice 1 §5 requires. The offline bundle and the
single-file HTML include the WASM builder, so the view works with no server.

## 9. Alternatives

| Alternative                                         | Verdict  | Why                                                                                                                                 |
| --------------------------------------------------- | -------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| SVG with one element per node                       | Rejected | Fine at 200 nodes, slow to re-layout and animate at 2000, and its accessibility is no better than a real tree list beside a canvas. |
| WebGL                                               | Deferred | Not needed at a 2000-node cap; worth it only if Drew asks for whole-graph display (slice 1 §9 question 1).                          |
| A web graph library (Cytoscape, Sigma, force-graph) | Rejected | Each brings its own layout and data model; the model owns layout here. Drawing code is small enough to own.                         |
| Building frames in TypeScript in the browser        | Rejected | A second implementation of the builder would drift from the server's and from slice 3's; one Rust crate serves all three.           |
| Plain click for `focus`, a button for `open`        | Rejected | Slice 1 fixed plain click as `open`, and people expect a click on a named thing to go to it.                                        |

## 10. Open questions

1. **Whole-graph display** stays Drew's call (slice 1 §9 question 1). Nothing in
   this slice assumes it; WebGL is the deferred path if he asks.
2. **Where the graph view lives in the app.** A route of its own (`/graph`) is
   reserved; whether it is also embedded in object pages as a small "nearby"
   panel is for the first implementation slice to try.

## 11. What slices 3 and 4 inherit

- The gesture-to-intent table (§5) as the pattern slice 3 follows for terminal
  mouse reporting; `focus` must not be a plain click there either.
- Keyboard order = floor order, which slice 3 reuses unchanged.
- The two orders a relaxed layout must keep, and the hit-test precedence
  (nodes over edges), as slice 4 checklist items.
- `web_reference.py` as the oracle for projection, hit-testing and keyboard
  order.
