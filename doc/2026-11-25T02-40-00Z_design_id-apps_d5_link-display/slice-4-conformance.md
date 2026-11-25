# D5: the link display system, slice 4 (fallback and conformance)

Design record for #13888 (parent epic #13880, programme order 5), slice 4 of 4,
the last. It closes the design: slice 1 ([`note.md`](note.md)) fixed the view
model, the contract and the list/tree floor; slice 2
([`slice-2-web.md`](slice-2-web.md)) the web and WASM renderer; slice 3
([`slice-3-kitty.md`](slice-3-kitty.md)) the Kitty terminal renderer. This
slice writes the checklist every renderer must pass and the one rule the floor
was missing.

Executable checklist: `fixtures/conformance.json`, run by
`fixtures/conformance_reference.py`
(`python3 -I fixtures/conformance_reference.py check`, stdlib only). It runs
every executable item against the three references and compares the floor of
every case with `fixtures/floor_expected.json` byte for byte. Today it reports
26 items, 22 executable and 4 manual, with 0 failures.

## 1. Problem

Three surfaces, written at different times by different people, will drift
unless something fails when they do. Slice 1 said the floor is "the conformance
floor every view must be able to produce"; this slice says exactly what
"conform" means, in a form an implementer can run.

## 2. Decisions

1. **The checklist is data.** `conformance.json` lists each item with an id, an
   area, the surfaces it applies to (`floor`, `web`, `kitty`), the record
   section it comes from, the requirement in one sentence, and either the name
   of an executable check or a manual procedure. An item with neither fails the
   run, and so does a check that no item names, so the list and the checks
   cannot drift apart.
2. **Implementations run the same checklist.** The Rust frame builder, the web
   renderer and the terminal renderer each get a test that loads
   `conformance.json`, filters by surface, and runs its own adapter for each
   executable item against the same fixtures. The Python references are the
   oracle the adapters are compared with, not the tests themselves.
3. **Golden output for the floor.** `floor_expected.json` holds the floor lines
   for all eight slice 1 cases. A floor implementation passes C-13 only when its
   bytes match, which also pins the line format of slice 1 §6.
4. **Manual items are few and concrete.** Four items need a person, a screen
   reader or a terminal with real image memory. Each has a procedure that says
   what to do and what must be seen. When a harness can automate one, the item
   gains a check and loses its procedure.

## 3. The floor, completed

Slice 1 §6 fixed the line format. One rule was missing: a label is user
content, and the floor prints it into terminals, logs and scripts. **Every
renderer sanitises a label before showing it:** each control character
(Unicode category Cc, including ESC, BEL, CR and LF), each line or paragraph
separator, and each bidi embedding, override or isolate (U+202A to U+202E,
U+2066 to U+2069) becomes U+FFFD. A label then cannot clear the screen, set the
clipboard through OSC 52, split a floor line or reverse the text around it.
The fixture `hostile_graph` in the reference carries such a title, and C-14
checks that none of it reaches the output.

The rest of the floor stands as slice 1 gave it: the spanning tree depth first
in angle order, two spaces per hop, the arrow, verb, label, kind, badges and
`(+N more)`, then "also linked", then "(node budget reached)". With no TTY the
output is those lines, each ended by a newline, with no escape bytes (C-15).

The floor is reached three ways, and all three are tested: as the no-TTY output
(C-15), as the terminal fallback when the probe does not confirm graphics
(C-16), and as the accessible tree beside the web canvas (C-24).

## 4. The checklist

| Id   | Area     | Surfaces          | Requirement                                                                                     | Test   |
| ---- | -------- | ----------------- | ----------------------------------------------------------------------------------------------- | ------ |
| C-01 | model    | floor, web, kitty | The frame builder produces the golden frame for every case.                                     | check  |
| C-02 | model    | floor, web, kitty | Every frame keeps the slice 1 invariants.                                                       | check  |
| C-03 | privacy  | floor, web, kitty | An unreadable object is never drawn, listed or counted in `+N more`.                            | check  |
| C-04 | privacy  | floor, web, kitty | An intent outside the frame is refused with `target_not_in_frame`.                              | check  |
| C-05 | layout   | web, kitty        | Ring order and clockwise angle order hold on screen, after any relaxation.                      | check  |
| C-06 | keyboard | floor, web, kitty | Keyboard order is the floor's line order.                                                       | check  |
| C-07 | keyboard | floor, web, kitty | All five intents are reachable from the keyboard.                                               | check  |
| C-08 | intents  | web, kitty        | A plain click opens where the pointer can hover; without hover the first click identifies.      | check  |
| C-09 | intents  | web, kitty        | `focus` is never the plain click.                                                               | check  |
| C-10 | intents  | floor, web, kitty | The four looking intents never change the frame.                                                | check  |
| C-11 | intents  | floor, web, kitty | Claimed times are shown as claims; a time-filtered frame says so.                               | check  |
| C-12 | intents  | floor, web, kitty | `open` on an edited node goes to the head form.                                                 | check  |
| C-13 | floor    | floor             | The floor matches `floor_expected.json` byte for byte.                                          | check  |
| C-14 | floor    | floor, web, kitty | Labels are sanitised (§3).                                                                      | check  |
| C-15 | floor    | floor, kitty      | No TTY prints the floor lines with no escape bytes.                                             | check  |
| C-16 | fallback | kitty             | Each probe profile chooses the named surface and mouse mode.                                    | check  |
| C-17 | fallback | kitty             | Teardown deletes every image and resets every mode, on every path.                              | check  |
| C-18 | fallback | kitty             | Graphics payloads are chunked at 4096 base64 characters with `m=` continuation.                 | check  |
| C-19 | pointer  | web, kitty        | A pointer at a node's centre hits the node, never an edge.                                      | check  |
| C-20 | pointer  | floor, web, kitty | Every node can be reached by pointer or keyboard.                                               | check  |
| C-21 | scale    | floor, web, kitty | No frame exceeds 2000 nodes; a cut says so and leaves `+N more`.                                | check  |
| C-22 | motion   | web, kitty        | Refocus keeps node identities for the transition.                                               | check  |
| C-23 | motion   | web, kitty        | Reduced motion swaps the frame at once and keeps the selection highlighted.                     | manual |
| C-24 | access   | web               | The floor is a `role="tree"` list beside an `aria-hidden` canvas, same order, shared selection. | manual |
| C-25 | access   | web, kitty        | Dimmed context is fainter and never takes label space before a bright node on its ring.         | manual |
| C-26 | fallback | kitty             | A runtime graphics error moves the view to the floor after a clean reset.                       | manual |

`conformance.json` holds the full wording, the source section of each item and
each manual procedure.

## 5. How an implementation uses it

1. Add a test per surface that reads `conformance.json` and keeps the items
   whose `surfaces` include that surface.
2. For each executable item, implement an adapter with the same name as the
   reference check. The adapter drives the implementation (the Rust builder,
   the web component in a headless browser, the terminal renderer against a
   transcript harness like #786's) and returns problems in the same shape.
3. Fail the test when an applicable item has no adapter, so a new item cannot
   be skipped silently.
4. List the manual items in the release notes of the first version of each
   surface, with who ran each procedure and what they saw.

A surface conforms when every executable item for it passes and every manual
item for it has a recorded run. A change to any record in this folder that
changes behaviour adds or edits an item here in the same pull request.

## 6. Alternatives

| Alternative                                | Verdict  | Why                                                                                                                              |
| ------------------------------------------ | -------- | -------------------------------------------------------------------------------------------------------------------------------- |
| A prose checklist only                     | Rejected | Nothing fails when a renderer drifts or when the list and the tests disagree.                                                    |
| Pixel snapshots of the web and the picture | Rejected | Fragile across fonts, GPUs and terminals, and they test drawing taste rather than the contract. Orders and hits are what matter. |
| Strip unsafe label characters silently     | Rejected | A reader then cannot tell that a label was altered; U+FFFD shows where something was removed.                                    |
| Escape labels as `\x1b` text               | Rejected | Readable to developers, noise to everyone else, and it changes line widths more than one replacement character does.             |

## 7. Open questions

1. **Whole-graph display** remains Drew's decision (slice 1 §9 question 1).
   If he asks for it, C-21's cap changes and the checklist gains scale items for
   the new bound.

## 8. Status of the design

All four slices are written, each with an executable reference, and together
they meet the issue's acceptance: both use-case graphs are expressed in the
model (slice 1), the contract is mapped feature by feature onto the web (slice 2) and onto Kitty with the fallback triggers named and #13623 cited (slice 3),
and the checklist is written so an implementer can test against it (this
slice). Building the renderers is implementation work for the issues that
follow from this record.
