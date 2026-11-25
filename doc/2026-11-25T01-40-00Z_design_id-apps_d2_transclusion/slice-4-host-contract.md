# D2: transclusion and embedding, slice 4 (host rendering contract)

Design record for #13887 (parent epic #13880), slice 4 of 4. It builds on
slice 1 in [`note.md`](note.md), which fixed the `embed` link, live and
snapshot modes, the six render states (§5) and the bounds (§6). Slice 2, the
full capability matrix, is still open; nothing here depends on how it lands,
because every rule below only narrows what a host may show.

Executable checklist: [`fixtures/host_contract.json`](fixtures/host_contract.json),
run by [`fixtures/host_contract_reference.py`](fixtures/host_contract_reference.py)
(`python3 -I fixtures/host_contract_reference.py check`, stdlib only). It runs
every executable item against the slice 1 reference renderer, its fixtures and
a few mutations. Today it reports 17 items, 13 executable and 4 manual, with 0
failures. The format matches the D5 conformance checklist
([`../2026-11-25T02-40-00Z_design_id-apps_d5_link-display/slice-4-conformance.md`](../2026-11-25T02-40-00Z_design_id-apps_d5_link-display/slice-4-conformance.md)),
so a host that runs one can run the other with the same harness.

## 1. Problem

Chat, forum, media and the core graph app (#14037) will each draw embeds.
Slice 1 says which object goes in each slot and in which state. It does not say
what a host may assume, what it must never show, how it loads, or what a reader
can do with a slot. Without that, the same embed looks different in each app,
and a host can leak through layout what the render state was designed to hide.

## 2. Decisions

1. **The checklist is data.** As in D5, each item has an id, an area, the
   section here it comes from, one sentence of requirement, and either an
   executable check or a manual procedure. An item with neither fails the run,
   and so does a check no item names.
2. **The render tree is the only input.** A host draws a slot from the slot the
   renderer returns and from nothing else. It does not read the target object,
   its body or its metadata to decide how to draw an `unavailable` slot. That
   is what makes §3.2 enforceable: the tree itself carries nothing to leak.
3. **States are decided eagerly; bytes load lazily.** A host resolves every
   slot's state when it renders the source, from graph metadata alone, and may
   defer fetching bodies (images, long text) until the slot is on screen.
   Lazy loading never changes a state (H-13).
4. **`pending` is a host placeholder, not a render state.** It covers the time
   before a readable target's object arrives (§3.3). It looks the same whatever
   the eventual outcome, and it always ends in one of the six states.
5. **Cut slots are stubs that open, never inline expansion.** `cycle-cut`,
   `depth-cut` and `budget-cut` name a readable object and offer to open it as
   a new root render, with fresh bounds. A host never expands one in place,
   so no interaction can turn a bounded render into an unbounded one.
6. **One fixed wording table** (§3.8) for every state and badge, so the same
   embed reads the same in every app and to a screen reader.

## 3. The contract

### 3.1 States and shapes

Every slot is exactly one of `shown`, `unavailable`, `removed`, `cycle-cut`,
`depth-cut` and `budget-cut` (H-01). A host that meets a state it does not
know draws it as `unavailable`, the safe floor, so a newer renderer never makes
an older host show more.

Each state has a fixed set of members (H-02), listed in `slot_keys` in the
checklist:

| State                                  | Members                                                                                         |
| -------------------------------------- | ----------------------------------------------------------------------------------------------- |
| `shown`                                | `link`, `mode`, `scope`, `state`, `object`; maybe `kind`, `embeds`, `members`, `more`, `badges` |
| `unavailable`                          | `link`, `state`                                                                                 |
| `removed`                              | `link`, `mode`, `scope`, `state`, `object`                                                      |
| `cycle-cut`, `depth-cut`, `budget-cut` | `link`, `mode`, `scope`, `state`, `object`                                                      |

A collection member is either a rendered object (`object`, `kind`, maybe
`embeds`) or a member cut (`object`, `state: cycle-cut`).

### 3.2 What a host must never show

- **Unavailable is opaque** (H-03). An `unavailable` slot has no mode, scope,
  kind, title, author, size, thumbnail or timestamp, because the renderer gives
  it none and the host reads nothing else (decision 2).
- **Causes are indistinguishable** (H-04). Unauthorised, not stored and pruned
  give the same slot, link aside. The check renders all three and compares.
- **Cuts name only readable targets** (H-05). A cut slot carries `object`, so
  the renderer checks readability before it cuts; a host must keep that order.
- **Collections count only what the viewer can read** (H-06). Members and
  `more` together never exceed the readable members.
- **Same footprint for every unavailable slot** (H-14, manual). A host sizes an
  unavailable slot from a fixed per-host size, never from the target. Otherwise
  the size of the gap would tell a reader how big the hidden thing is.

### 3.3 Loading and sync

- Before a slot resolves, the host reserves one fixed placeholder per scope:
  one card for `object`, one card with a fixed number of rows for `collection`
  (the host picks the number; `max_count` is the embedder's statement and may
  be shown). Layout may shift once, when the slot resolves.
- **Not yet synced** (H-15, manual). When the viewer can read the target's
  area but the object is not stored locally, the host shows `pending` while
  sync runs. It shows the object if it arrives, and `unavailable` once the
  host's sync timeout passes. The value of that timeout is the host's; it must
  be finite.
- **No fetch without read.** The read check comes first. A viewer who cannot
  read the target gets `unavailable` at once and the host starts no fetch for
  it. The timing difference between "at once" and "after pending" only
  separates outcomes for objects the viewer can read, so it reveals nothing
  about objects the viewer cannot.
- **Bytes are lazy, states are not** (H-13). The check renders every case with
  every body removed and requires the same tree.

### 3.4 Bounds

- A render shows at most the host budget of objects (64 in the reference) and
  nests shown slots at most the default depth (3); the level below holds only
  cut or unavailable stubs (H-11). The check adds two stress graphs, a chain of
  ten notes and a collection of 200 members, to the slice 1 cases.
- A collection slot shows at most its `max_count` members (H-12), with the rest
  in `more`.
- A host may pick a smaller budget than the reference, never an unbounded one.

### 3.5 Modes and badges

- A shown snapshot object slot shows its pin, never the head (H-08). A host
  may offer "view current" beside it, and that opens the head as a new root
  render under the same read rules.
- A shown live object slot shows the viewer's readable supersede head (H-09).
- Badges are `edited`, `fork` and `changed-since-embed`, only on shown slots,
  each at most once (H-10).
- **Slice 1 open question 3, answered:** a collection snapshot badges added
  members only. A removed member is a readable nest event; a host that wants to
  show it opens the nest's history rather than marking the embed. No fourth
  badge.

### 3.6 Revocation and re-render

Losing read access turns a shown slot `unavailable` on the next render
(H-07). A host must not reuse a shown slot, a cached body or a thumbnail from
an earlier render for a viewer who can no longer read the object. A live slot
re-renders when its head moves; a snapshot slot re-renders only for badges and
access.

### 3.7 Interaction

H-16 (manual) holds these together:

- A shown slot opens its object by its `id://` URI (D1).
- A cut slot opens its object as a new root render. `cycle-cut` never expands
  inline; `depth-cut` and `budget-cut` say how to see more by opening.
- `more` opens the collection itself as a new root render.
- An unavailable slot has no action. Its context menu offers no copy-link, and
  the host never shows the target URI, even though the link object holding it
  is readable to anyone who reads the source.

### 3.8 Words for each state

H-17 (manual). Every state and badge has a fixed text, used as the visible
label of a stub and as the accessible name of a slot:

| State or badge        | Text                                                        |
| --------------------- | ----------------------------------------------------------- |
| `shown`               | the object's own label, sanitised as D5 slice 4 §3 requires |
| `unavailable`         | Unavailable                                                 |
| `removed`             | Removed                                                     |
| `cycle-cut`           | Already shown above. Open                                   |
| `depth-cut`           | Embedded too deep to show here. Open                        |
| `budget-cut`          | Too many embeds to show here. Open                          |
| `pending`             | Loading                                                     |
| `edited`              | Edited                                                      |
| `fork`                | Has more than one newer version                             |
| `changed-since-embed` | Changed since it was embedded                               |
| `more`                | and N more                                                  |

## 4. The checklist

| Id   | Area          | Requirement                                                                                         | Test   |
| ---- | ------------- | --------------------------------------------------------------------------------------------------- | ------ |
| H-01 | states        | Every slot is one of the six states; an unknown state is shown as unavailable.                      | check  |
| H-02 | states        | Each state carries exactly its listed members.                                                      | check  |
| H-03 | privacy       | An unavailable slot carries only its link and state.                                                | check  |
| H-04 | privacy       | Unauthorised, not stored and pruned give the same slot.                                             | check  |
| H-05 | privacy       | A cut slot names only a readable target.                                                            | check  |
| H-06 | privacy       | Members and `more` cover only readable members.                                                     | check  |
| H-07 | privacy       | Revoked access makes a shown slot unavailable on the next render.                                   | check  |
| H-08 | modes         | A shown snapshot object slot shows its pin.                                                         | check  |
| H-09 | modes         | A shown live object slot shows the readable head.                                                   | check  |
| H-10 | badges        | Badges are known, on shown slots only, never repeated.                                              | check  |
| H-11 | bounds        | Budget and depth bounds hold, with cut stubs only one level below.                                  | check  |
| H-12 | bounds        | A collection shows at most `max_count` members.                                                     | check  |
| H-13 | loading       | Withholding bodies changes no state.                                                                | check  |
| H-14 | loading       | Fixed footprint per scope; every unavailable slot is the same size.                                 | manual |
| H-15 | loading       | Readable but unsynced targets show pending, then shown or unavailable; unreadable ones never fetch. | manual |
| H-16 | interaction   | Shown opens; cuts open as a new root; unavailable has no action and shows no URI.                   | manual |
| H-17 | accessibility | Every state and badge reads with the fixed wording; unavailable reads the same for every cause.     | manual |

The checks were also run against deliberately broken renderers (an
unavailable slot that keeps its mode; see the PR) and fail as they should.
H-05 passes on today's fixtures because the reference checks readability
first; no fixture yet puts an unreadable target exactly at a cut, so a host's
own adapter test should add one.

## 5. What D5 and the host apps take from this

- `host_contract.json` and its checker, run by every host next to D5's
  `conformance.json`.
- The wording table (§3.8), which D5's floor uses for embed slots.
- Decision 2: the render tree is a host's only input for a slot.

## 6. Alternatives

- **Let hosts size unavailable slots from the target.** Nicer layout, but the
  gap's size leaks the hidden object's shape. Rejected (H-14).
- **Expand cut slots inline on click.** Convenient, but each click would grow
  one render past its bounds. Opening as a new root keeps every render bounded.
- **A `syncing` render state.** It would make the renderer depend on sync
  state and give a seventh state every host must draw. `pending` stays a host
  placeholder that always ends in one of the six.
- **A `members-removed` badge.** Answered no in §3.5.

## 7. Open questions

Slice 1's questions 1, 2 and 4 stand unchanged: admitting the `embed` verb
(Drew), render-bound snapshots (Drew, with D5), and the delegated scoped view
(slice 2). This slice answers question 3 (§3.5) and adds none.
