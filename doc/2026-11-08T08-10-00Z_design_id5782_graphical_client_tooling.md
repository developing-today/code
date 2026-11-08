# Graphical client & tooling surface — v1 design

Design record for issue #5782 ("id: design: graphical client & tooling
surface — windowing, world inspector, debug UI"). Written per
`doc/DOCUMENTATION_PROTOCOL.md` (dated record in `doc/`). Origin issue:
#5782, parent epic #2056. Constraining records: #788 (native client
shells on the shell contract — framework and conformance live there),
#1072 (end-user app idea and stack decision — this design is requirements
input to it, not a duplicate). Inputs already landed: #790 (multi-world
topology), #5545 (discovery query + counted/uncounted writer-set view
projection). Displayed state owned elsewhere: #2819 (lease state),
#5684 (delegation objects), #835 (moderation/write-affordance question),
#558 (dev-loop results). Status: v1 design, settled except the items
explicitly deferred in §7. No implementation is authorized by this
record alone; §9 names the implementation slices as next actions.

Verified at origin/main `e81491d4cf` (2026-11-08): the only graphical
presenters are the web presenter (`pkgs/id/web/src/`, including
`world.ts`) and the SSH/terminal presenters; there is no inspector or
debug-UI design or implementation in the tree, and no PR references
#5782. The journal/event substrate the inspector renders exists
(`pkgs/id/src/event_store/`, determinism fixtures under
`pkgs/id/tests/fixtures/`), and presence exists as data
(`pkgs/id/src/chat_presence.rs`) — what is missing is the surface
design, which is this record.

---

## 1. Problem

#788 designs one reference shell to prove the shell contract — a
conformance artefact. It does not own the graphical surface people
will actually work in: multi-world windowing and presence, the world
inspector (journal, triple graph, capability grants, lease state as
browsable objects), or the debug UI (event tailing, replay scrubbing,
fuzz/soak playback). Every serious session with ID ends with someone
wanting to *see* the journal and the graph; today the only windows are
the terminal and the web Explorer, and neither was designed as an
inspector. This record designs that surface as requirements any shell
(web growth or native, per #788/#1072) must satisfy identically where
§6 says so.

## 2. Proposal in one paragraph

One surface, three components, in the same app as the world view:
**windowing & presence** (§3) organises many worlds and servers around
the #5545 discovery query rendered as a world list; the **inspector**
(§4) is a set of authority-gated panels in the same app — journal
browser, entity/triple explorer, grant/capability viewer, and the
counted/uncounted writer-set sides side by side; the **debug UI** (§5)
adds a live event tail, a replay scrubber over the determinism
fixtures, and fuzz/soak failure playback. The inspector is read-only
by design in v1 (§7 Q4). Everything the surface renders comes through
bounded, paged presenter read models (§8); no new write path is
created for it.

## 3. Windowing & presence

- **One client, many worlds, many servers.** The app holds concurrent
  sessions to several servers; a world is always rendered in the
  context of the server that hosts it. Switching worlds never tears
  down another world's session or subscriptions.
- **World organisation.** Worlds are organised as a list with pinned
  worlds first, then recent, then discovered. Discovery is the #5545
  discovery query *rendered*: the same counted/uncounted distinction
  the query returns is visible in the list (a world you can find but
  are not a counted writer of is shown as such, never silently
  omitted and never shown as fully joined).
- **Window model.** One primary window per app instance; the world
  view and inspector panels share it (§7 Q1). Native shells may
  detach panels into separate OS windows; web shells render the same
  panels as docked regions. Detaching never changes what a panel
  shows — it is a layout act, not a mode.
- **Presence, graphically.** Presence answers three questions at a
  glance: who is in this world, where in the journal they are, and
  what they are doing (viewing, editing, replaying). It renders from
  the existing presence data (`chat_presence` and its successors):
  avatars/markers in the world header, per-panel presence for shared
  inspector views (two operators looking at the same journal range
  see each other), and no presence at all for worlds the user only
  discovers (presence is a counted-side signal; the uncounted side
  shows aggregates or nothing, per #5545 semantics).

## 4. The inspector

The inspector is four panels over presenter read models (§8). Every
panel is deep-linkable (world + panel + object/range) so a journal
position or a grant can be pasted into an issue or chat and opened
identically by another operator.

- **Journal browser.** The journal as a browsable table: position,
  kind, author, epoch, and payload summary, with filters on each.
  Lease state is a first-class column family, not a payload detail:
  current lease holder, fencing/epoch information, and handoff
  history (#2819) render as rows in the same browser, so "who held
  the write lease when this entry landed" is a filter, not a
  forensic exercise. Selecting an entry shows its full payload and
  its position in any replay (§5).
- **Entity/triple explorer.** Following the graph from any entity:
  outbound and inbound triples, typed, with the journal entries that
  asserted them one click away. Bounded by design (§7 Q2): the
  explorer renders a neighbourhood, never "the graph".
- **Grant/capability viewer.** Who can do what, issued by whom, under
  which delegation: #5684's delegation objects rendered as chains
  (root grant → delegation → exercise), each link showing scope,
  expiry, and revocation state. The viewer answers both directions:
  "what can this principal do here?" and "who can do this action
  here, and by what chain?".
- **Writer-set sides.** The counted and uncounted writer sets shown
  side by side, always: "uncounted but findable" is a visual idea
  and this panel is where it lives. Divergence between the sides
  (an uncounted writer whose writes are landing) is highlighted as
  a state of the world, not an error banner.

## 5. Debug UI

- **Live event tail.** Per world, a tail of journal entries as they
  land, pausable, filterable by kind/author, with the tail position
  pinned to the journal browser (pause the tail, open that exact
  range in the browser). High-rate worlds degrade to sampled tail
  with the sampling stated on screen — never a silently lossy tail.
- **Replay scrubber.** Deterministic replay as a transport control:
  pick a journal range (or a determinism fixture), scrub position,
  step forward/back, and watch projected state rebuild. Replay is a
  read-only projection over recorded entries and fixtures
  (`pkgs/id/tests/fixtures/` and the event-store replay path); it
  never writes to a live world. The scrubber position and the
  journal browser selection are the same coordinate system.
- **Fuzz/soak playback.** A recorded fuzz or soak failure is an
  artefact the debug UI can play: the failing input sequence stepped
  through the scrubber, with the divergence point (expected vs
  actual state) marked. Dev-loop results from #558 surface here as
  playable artefacts in the client, not only as terminal text.

## 6. Surface split

Decided here as requirements; framework choice stays with #788's
record and #1072's stack decision.

| Surface element | Web (Explorer growth) | Native | Identical in both |
|---|---|---|---|
| World list + discovery rendering (#5545) | yes | yes | semantics, counted/uncounted rendering |
| Presence header | yes | yes | presence model and states |
| Journal browser | yes | yes | filters, lease columns, deep links |
| Triple explorer | yes, within §7 Q2 budgets | yes, within §7 Q2 budgets | budgets, truncation affordance |
| Grant/capability viewer | yes | yes | chain rendering, both query directions |
| Writer-set sides | yes | yes | side-by-side rule |
| Live event tail | yes, sampled at lower rate cap | yes | sampling disclosure rule |
| Replay scrubber | yes, fixture/range replay | yes | coordinate system, read-only rule |
| Fuzz/soak playback | yes | yes | divergence marking |
| Multi-window detach | docked regions instead | yes | panel content identical either way |
| 3D / spatial world rendering | no (out of Explorer scope) | yes, per #788 shell | shell contract events only |

The "identical" column is the conformance surface: a shell that
renders one of these elements renders it with these semantics, which
is what #788's conformance check should grow to cover for the
graphical layer.

## 7. The four open questions — decisions

**Q1. Inspector: mode, separate app, or authority-gated panels?**
DECIDED: the same app, with authority-gated panels. Not a separate
app (the inspector's whole value is zero context switch from the
world it inspects; a separate app also forks the conformance
surface), and not a mode (modes hide the world while you inspect
it). Panels are gated by capability: a principal without the
relevant grant does not see the panel at all — hidden, not disabled —
so the shipped surface and the operator surface are one artefact
with different authority, and screenshots never leak the existence
of gated state.

**Q2. How much graph visualisation is honest at world scale?**
DECIDED: the explorer is a neighbourhood tool with hard budgets —
default radius 2 from the focus entity, at most ~500 visible nodes,
with aggregation (typed supernodes with counts) as the budget is
approached and an explicit "neighbourhood truncated at budget —
refine by type or follow a specific edge" affordance when it is
hit. Beyond the budget the honest views are list and matrix, and
the explorer offers them instead of silently sampling. A graph
picture that samples without saying so is a false statement about
the world; that is the failure this budget exists to prevent.

**Q3. Accessibility and keyboard-first operation as constraints?**
DECIDED: constraints, from v1, not afterthoughts. Every graphical
element in §6 has a list/table equivalent with the same information
(the panels above are specified as tables first for this reason);
the full surface is operable keyboard-only with a documented focus
model; the event tail's screen-reader announcements are throttled
and pausable with the tail itself; contrast and reduced-motion
preferences are honoured by the presence and tail animations. An
implementation slice (§9) does not pass review without its keyboard
path and list equivalent.

**Q4. Write affordances, or read-only by design?**
DECIDED for v1: read-only by design. The inspector creates no write
path of its own. Moderation and other writes (the #835 question)
enter, if #835 decides they should, as *proposed-action drafts*
composed in the inspector but executed through the ordinary client
write path — with that path's own authority checks, journal
entries, and lease rules — never as a privileged inspector channel.
DEFERRED: whether proposed-action drafts are built at all. Deciding
evidence: #835's moderation-surface ruling and the first operator
need that read-only inspection demonstrably cannot serve. Owner:
#835, with this design as its input.

## 8. Read models (presenter contracts)

The surface needs six bounded presenter read models. All are paged
or budgeted, all state their truncation, and none is a new write
API. Names are logical; the owning presenter (web first, per §6)
shapes the concrete encoding.

| Read model | Contents | Bound |
|---|---|---|
| `world_directory` | discovery results with counted/uncounted sides, pins, recency | paged |
| `journal_page` | entries by position range, kind/author/epoch filters, lease columns | paged by position |
| `triple_neighbourhood` | focus entity, radius, typed edges, aggregate supernodes | §7 Q2 budgets |
| `grant_chains` | chains for a principal or an action, scope/expiry/revocation per link | paged |
| `event_tail` | live subscription at journal position, sampling state disclosed | rate-capped, sampling stated |
| `replay_session` | projection rebuild over a range or fixture, step/scrub positions | range-bounded, read-only |

## 9. Implementation slices (next actions)

Named here so the design does not sit silently; each slice becomes
its own issue under epic #2056 when the epic is next groomed, and
the web presenter is the reference implementation for each
(native follows under #788 conformance).

1. **World directory panel** — #5545 discovery rendered, pins,
   presence header. (§3)
2. **Journal browser with lease columns** — filters, #2819 lease
   and handoff rows, deep links. (§4)
3. **Grant/capability viewer** — #5684 chains, both directions. (§4)
4. **Writer-set sides panel** — counted/uncounted side by side. (§4)
5. **Triple explorer within budgets** — neighbourhood, supernodes,
   truncation affordance, list/matrix fallback. (§4, §7 Q2)
6. **Debug UI: tail + replay scrubber + failure playback** —
   event tail with disclosed sampling, fixture/range replay,
   #558 artefacts playable. (§5)
7. **Graphical conformance growth** — #788's conformance check
   extended to §6's identical column. (§6)

## 10. Non-goals

- Implementing the graphical client or inspector (named in §9,
  owned by its own slices).
- Choosing a framework or changing the shell contract (#788), or
  re-deciding the end-user app stack (#1072).
- Moderation policy and any privileged write path (#835; §7 Q4).
- New discovery, lease, or delegation semantics — this design
  renders #5545, #2819, and #5684 as they are.
