# D5: the link display system, slice 3 (Kitty terminal renderer)

Design record for #13888 (parent epic #13880, programme order 5), slice 3 of 4.
It builds on slice 1 ([`note.md`](note.md): the frame, the polar layout, the
five intents and the list/tree floor) and slice 2
([`slice-2-web.md`](slice-2-web.md): the projection, hit-testing, the gesture
table and keyboard order). Prior art: #13623, the presenter-side Kitty graphics
stills pipeline, and its research
([`2026-11-23T18-32-12Z_research_id5971_image_video_conversion_tooling/`](../2026-11-23T18-32-12Z_research_id5971_image_video_conversion_tooling/));
the capability tiers of #786
([`2026-11-01_design_id786.md`](../2026-11-01_design_id786.md)) and their probe
in `pkgs/id/crates/id-worlds/src/world_terminal.rs`.

Executable reference: `fixtures/kitty_reference.py`
(`python3 -I fixtures/kitty_reference.py check`, stdlib only). It chooses the
surface for ten probe profiles and checks each named fallback trigger, parses
SGR mouse reports, hits every node of every slice 1 frame from cell and pixel
reports, turns six timed mouse scripts into intents, chunks graphics commands
and round-trips them, places labels in cells without overlap, and checks that
teardown restores every mode it set.

## 1. Problem

Drew asked for the graph view in terminals, "especially Kitty-protocol
terminals, with mouse hover and click". A terminal is a cell grid that may or
may not draw pictures, may or may not report the mouse, and may sit behind tmux
or SSH. The renderer must give the same picture and the same contract as the
web where it can, and the list/tree floor where it cannot, and it must never
leave a broken screen.

## 2. Decisions

1. **The picture is a raster drawn by the presenter, placed with the Kitty
   graphics protocol; labels are terminal text.** Nodes and edges are drawn
   into one PNG at the pane's pixel size, on the slice 2 projection, and placed
   under the text layer (`z=-1`). Labels are written as ordinary characters in
   the cells beside each dot. Text stays crisp in the user's font, can be
   copied, and needs no font rendering in the presenter.
2. **The same projection as the web.** The pane's pixel size is its cell count
   times the probed cell size, and `project` from slice 2 places every node.
   Pixels are square, so no aspect correction is needed; slice 1 §4 left that
   question here, and the answer is that cells are only a hit-testing grid.
3. **Surface choice is by positive evidence, with one named reason per
   fallback** (§3). Without graphics the renderer draws the list/tree floor,
   interactive, never a degraded picture.
4. **A picture without a mouse is still a picture.** Every intent is reachable
   from the keyboard (slice 1 §5), so a terminal that draws graphics but does
   not report the mouse gets the picture with keyboard control. The issue's
   wording sends "without graphics or mouse" to the floor; this slice keeps the
   floor for the no-graphics case and treats a missing mouse as a reason to
   turn mouse reporting off, not to throw the picture away.
5. **Presenter-owned bytes only.** As #786 requires, the terminal adapter is the
   only component that writes terminal control output; the graph view is a
   presenter surface, and nothing in a frame (labels included) reaches the
   terminal unescaped. Labels are stripped of control characters before they
   are written.
6. **One encoder with #13623.** The APC framing, the `a=q` gate, chunking and
   image ids come from the #13623 pipeline (little-kitty per the research's
   amendment A.3); this renderer adds placement, overlays and teardown on top.

## 3. Detection and fallback triggers

The #786 probe already confirms Kitty graphics, synchronized output and
indexed colour. This renderer needs four more facts, each from its own reply,
added to the same single-deadline probe:

| Fact              | Query                                | Used for                       |
| ----------------- | ------------------------------------ | ------------------------------ |
| `png_accepted`    | `a=q,f=100` with a small PNG, `t=d`  | Detection alone is not support |
| `cell_pixels`     | `CSI 16 t` (reply `CSI 6 ; h ; w t`) | Pane pixel size and hit-tests  |
| `mouse_sgr`       | DECRQM `?1006` recognised            | Unambiguous mouse reports      |
| `mouse_any_event` | DECRQM `?1003` recognised            | Hover (motion with no button)  |
| `mouse_pixels`    | DECRQM `?1016` recognised            | Pixel-precise reports          |

The surface follows from the profile, checked in this order:

| Reason                   | Surface | When                                                                   |
| ------------------------ | ------- | ---------------------------------------------------------------------- |
| `no_tty`                 | lines   | Not an interactive terminal: print the floor once and exit.            |
| `probe_conflicted`       | floor   | Contradictory complete replies (the #786 conservative rule).           |
| `no_kitty_graphics`      | floor   | No graphics reply, including tmux without passthrough.                 |
| `query_refused`          | floor   | The `a=q` PNG query was refused or not answered in the deadline.       |
| `no_synchronized_output` | floor   | Without DEC 2026 a redraw can tear mid-frame.                          |
| `cell_size_unknown`      | floor   | No cell pixel size, so the picture cannot be lined up with text cells. |
| (none)                   | picture | Every fact above was confirmed.                                        |

Mouse reporting is chosen separately: `hover` when SGR and any-event motion are
both confirmed, `click` when only SGR is, `keyboard` otherwise. The floor uses
the same mouse modes on its lines.

Two runtime triggers move a running picture to the floor after a clean reset:
the terminal answers a transmission with an error (for example `ENOSPC` or
`EINVAL` when it runs out of image memory), or a write to the terminal fails.
A resize re-probes the cell size; a session reattached through tmux re-runs the
whole probe, because the outer terminal may have changed. Sixel-only terminals
get the floor in v1; sixel has no placements or deletes by id, so hover
overlays would mean redrawing the whole picture.

## 4. Drawing and redraw discipline

Each frame is drawn inside synchronized output (`CSI ? 2026 h` … `l`):

1. **The picture.** Transmit and place one PNG (`a=T,f=100,t=d`) with a fixed
   image id and placement id, covering the pane in cells (`c=`, `r=`),
   `z=-1`, `C=1` so the cursor does not move, and `q=2` so the terminal sends
   no replies into the input stream. Payloads are base64 in chunks of at most
   4096 characters with `m=1` on all but the last; control keys go on the first
   chunk only. Over SSH or iroh only the direct medium (`t=d`) exists, so the
   renderer never uses files or shared memory.
2. **Labels.** Each node's label goes in the cells right of its dot, cut where
   the next label or another dot begins, with an ellipsis. The focus claims
   cells first, then bright nodes by hop, then dimmed ones; a node with no room
   keeps its label in the identify card and the floor. Dimmed labels use the
   terminal's faint attribute.
3. **Hover highlight is an overlay.** Highlighting a node's edges and
   neighbours does not retransmit the picture: a second image id holds a
   transparent RGBA overlay (`f=32,o=z`) cropped to the highlighted region,
   placed above the picture and below text. It is replaced on each hover change
   and deleted when the hover leaves.
4. **Coalescing.** Motion reports arrive faster than a terminal can take
   pictures. The renderer keeps only the newest hover target and redraws at
   most 60 times a second (the #786 Tier 1 ceiling); a frame change always wins
   over a pending hover.
5. **Transitions.** A `focus` replaces the picture. With synchronized output
   the swap is atomic; the 250 ms interpolation of slice 2 is drawn as at most
   six intermediate pictures, and none when the renderer's reduced-motion
   setting is on.
6. **Zoom and pan** use the slice 2 rules: `+`/`-` or Ctrl+wheel zoom around
   the pointer, the wheel and the arrow keys with Shift pan, `0` fits.

Teardown runs on every exit path, normal, error and disconnect, as #786
invariant 7 requires for synchronized output: delete both images by id
(`a=d,d=I`), then turn off every mode in reverse order (any-event and button
mouse reporting, pixel reports, SGR encoding, the hidden cursor, the alternate
screen). The reference checks each enabled mode has its matching reset.

## 5. Mouse and keyboard mapping

Reports are SGR (1006): `CSI < b ; x ; y M` for a press or motion and `m` for a
release, with bit 32 for motion, bit 64 for the wheel and bits 4, 8 and 16 for
Shift, Alt and Ctrl. With pixel reports (1016) the coordinates are pixels;
otherwise the renderer takes the centre of the reported cell, and widens the
node hit radius to half a cell's diagonal so a click on the cell that holds a
dot always hits a node in that cell. When two dots share a cell, the nearer one
wins and the other is reachable by zooming in or from the keyboard. Nodes win
over edges, as on the web.

| Mode     | Report                       | Intent                                        |
| -------- | ---------------------------- | --------------------------------------------- |
| hover    | Motion onto a node           | `identify`                                    |
| hover    | Motion onto an edge          | `identify-link`                               |
| hover    | Click a node                 | `open`                                        |
| hover    | Click an edge                | `inspect-link`                                |
| hover    | Double click a node          | `focus`                                       |
| click    | Click a new target           | `identify` or `identify-link`, and a card     |
| click    | Click the selected target    | `open` or `inspect-link`                      |
| click    | Double click a node          | `focus`                                       |
| any      | Click Open, Focus or Inspect | That intent, from the card in cells           |
| keyboard | Arrows                       | `identify` (move the selection)               |
| keyboard | `l` or Tab                   | `identify-link` (cycle the selection's edges) |
| keyboard | Enter                        | `open`                                        |
| keyboard | `i`                          | `inspect-link`                                |
| keyboard | `f`                          | `focus`                                       |

`click` mode is the terminal form of touch in slice 2: with no motion reports
there is no hover, so the first click identifies. A click that might be the
first of a double click waits the double-click interval (400 ms unless the
terminal reports its own) before `open` acts, so a double click never also
opens. Shift+Enter, the web's `inspect-link` key, is not distinguishable from
Enter without the Kitty keyboard protocol, so the terminal uses `i`.

Keyboard order is the floor's line order, as in slice 2: Down and Up walk the
spanning tree depth first in angle order, Left goes to the parent and Right to
the first child.

`open` and `inspect-link` show the object's page inside the same terminal
session (Backspace returns to the graph). Where the terminal supports OSC 8
hyperlinks, the page title carries the object's web address, so a person can
also open it in a browser.

The identify card is drawn in cells, below the picture: label, kind, author,
claimed time marked as claimed, badges, a one-line preview and, in `click`
mode, Open, Focus and Inspect buttons.

## 6. The floor surface

When the picture is not available, the renderer shows the slice 1 floor lines,
interactive: arrows move the selection, Enter opens, `f` focuses, `l` cycles
the selected line's edges, `i` inspects. With mouse reporting, a click on a
line selects it (or opens it, in hover mode), so the floor honours the same
contract. With no TTY, the floor lines are printed once, exactly as slice 1 §6
gives them, which is also the scripting form.

## 7. Prior art: #13623

#13623 builds the stills half of this renderer: the bounded image decode, PNG
passthrough or RGBA plus zlib, 4096-character chunking with `m=`, image ids and
placements, and the `a=q` gate, with golden fixtures against icat and chafa.
This slice reuses all of it unchanged and adds four things #13623 does not
need: a second image for overlays, deletes by id on frame change and teardown,
cell-aligned labels, and mouse reporting. The research's support matrix
(§2.3) is why the probe, not the terminal's name, decides: kitty and Ghostty
take the picture, WezTerm takes stills (no animation is needed here), Konsole
answers the query only for a subset, and tmux passes graphics only with
passthrough.

## 8. Alternatives

| Alternative                                         | Verdict  | Why                                                                                                                                                                     |
| --------------------------------------------------- | -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Draw the graph in cells (box and braille lines)     | Deferred | Works without graphics, but crossing edges become unreadable past a few dozen nodes, and it is a fourth drawing to keep in step. The floor is clearer at the same cost. |
| Labels drawn into the picture                       | Rejected | Needs a font in the presenter, blurs at small sizes, cannot be copied, and every hover redraw would retransmit text pixels.                                             |
| Unicode placeholders for the picture                | Deferred | They survive scrollback and reflow, which a full-screen view does not need; they are the right tool when a graph is embedded in a note.                                 |
| Redraw the whole picture on hover                   | Rejected | A 1200×800 PNG per motion report saturates SSH; the overlay costs only the highlighted region.                                                                          |
| Sixel                                               | Deferred | No placement ids or deletes; hover would redraw everything. The floor serves sixel-only terminals in v1.                                                                |
| Fall back to the floor whenever the mouse is absent | Rejected | Every intent is keyboard-reachable, so the picture still meets the contract; throwing it away would cost the people with the least capable terminals the most.          |

## 9. Open questions

1. **Graphs inside notes.** Embedding a small graph in a note's terminal view
   wants Unicode placeholders and a non-full-screen placement. That belongs with
   D2's embed renderers, not this full-screen view.
2. **Cell-grid drawing.** If people on terminals without graphics ask for a
   picture, a cell renderer can be added behind the same projection; slice 4's
   checklist would then test it as a third graphic surface.

## 10. What slice 4 inherits

- The fallback triggers in §3, each a checklist item with a probe transcript.
- Teardown: every mode enabled is reset, every image deleted, on every path.
- Mouse mapping for `hover`, `click` and `keyboard` modes, and the hit rule
  for cell reports.
- `kitty_reference.py` as the oracle for surface choice, SGR parsing, gesture
  timing, chunking and label placement.
