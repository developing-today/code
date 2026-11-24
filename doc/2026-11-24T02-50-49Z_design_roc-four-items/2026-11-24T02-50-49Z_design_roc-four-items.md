# Roc four items — direction appended to the composed-platform design

**Date:** 2026-11-24 · **Lane:** ROC-FOUR-DIRECTIONS
**Direction:** Drew, 2026-11-23 ~20:48 CST (implement and use each of Rocco,
Roclet, concave, and roc-pdf; build an example app for each one, or
multiple).
**Amends:** "Roc Platforms Deep-Dive + A Composable Platform Family for ID
Apps" (the ROC-PLATFORMS-DESIGN exploration, 2026-11-23), as amended by the
platform composer record
(`doc/2026-11-24T02-45-34Z_design_roc-platform-composer/`, PR #14029). The
design exploration's repo copy lands as
`doc/2026-11-24T02-09-38Z_research_roc-investigation/platform-design-exploration-2026-11-23.md`
via PR #14012. The identical text below is appended as §3.9 of the working
note in `~/workspace/roc-investigation/`, directly beside §3.8 (the
own-the-five-platforms direction).
**Programme context:** the wider frame is the ID design programme for worlds,
capabilities and permissions (#5955); this direction belongs to the platform
design record above, not to a separate design.
**Open question carried:** §3.9 item 3 records a named question for Drew —
whether "implement" for concave means building our own implementation of
its shape or adopting the days-old upstream as the base of our variant. It
is flagged there, not decided.

The direction follows, verbatim as appended to the design note.

---

## 3.9 Direction recorded (Drew, 2026-11-23, later): the four join the family

A second direction from Drew the same evening, extending §3.8's list:

**Drew's direction, in substance:** _"Plan to implement and use each of
these and build an example app for each one, or multiple."_

"These" are the four items from the decisions-lane report — **Rocco**,
**Roclet**, **concave**, and **roc-pdf**. They join the family plan
alongside the five platforms of §3.8, but they are not four more of the
same thing, and this entry keeps their shapes distinct:

1. **Rocco — a platform in the same sense as the five.** A 3D engine
   platform (engine in Odin, WebGPU rendering; a game is a model plus five
   pure functions returning a `Scene` value; PBR, shadows, glTF, skinned
   animation, collision queries, no physics; web builds via wasm +
   WebGPU). It is a candidate for a variant of ours under §3.8's logic,
   and its capability surface (scene/render, input, audio) is a candidate
   cap family for the catalogue. Two months old, 21 releases, explicitly
   early — the youngest of the platform candidates, and the one where
   "implement" most plausibly means growing our variant alongside a fast
   upstream rather than stabilising a finished thing.
2. **Roclet — also a platform in the same sense.** Notebooks for Roc
   running entirely in the browser via Wasm: reactive cells ordered by
   dependency rather than position, notebooks saved as plain Roc or
   Markdown, exportable as runnable scripts or HTML reports. Experimental
   alpha, Chrome only. A variant candidate like Rocco; its example apps
   are example _notebooks_, which in its model are also its distributable
   artefact.
3. **concave — a platform, but first a reference design.** The
   Convex-style reactive backend announced the same day as the report:
   typed Roc queries and mutations, queries as live subscriptions by
   default, ACID mutations that can schedule jobs transactionally,
   built-in document database, file storage, auth integrations, an
   auto-generated typed client, pure-Roc components. Days old, zero
   releases, no production users, and its formal-verification claims are
   the author's own. Its certain role is as the reference point for the
   id backend shape — it proves the reactive-backend slice is buildable
   on RC1, and its author's compiler-API workaround (a manual string
   registry plus an `--introspect` mode) is the first use case for the
   embedding adapter the decisions lane recommended. Whether it is also
   the base code of our backend platform is **an open question for Drew,
   recorded here rather than decided:** "implement" can mean _(a)_ build
   our own implementation of the concave shape on the id platform family,
   or _(b)_ adopt the days-old upstream as the base of our variant, per
   §3.8's pattern for the five. The costs differ by an order of
   magnitude, and the use-and-example-app parts of the direction apply
   either way.
4. **roc-pdf — a library, not a platform.** Boswell's Roc library that
   _generates PDF files_ — pagination, font embedding, deterministic
   standards-conformant output — with twelve releases and the most mature
   footing of the four (Åhdén generates real-estate sale flyers with it in
   production). It joins the family as a library: nothing here gets a
   variant in the platform sense and it takes no catalogue entry, because
   catalogue caps are host capabilities and a pure Roc library needs
   none. Example apps consume it _through_ a platform — a server- or
   CLI-profile app that generates documents. "Implement and use" in
   library form: depend on it, and own fixes in a library variant if it
   becomes load-bearing.

**The example-app programme shape.** At least one example app per item,
multiple where the item supports it: Rocco and roc-pdf support several
naturally (small games and visual tools; document generators of different
kinds), Roclet's are notebooks, concave starts with one reactive app.
Example apps are built against our variants (for roc-pdf, against the
library through one of our platforms) on the family's pinned nightly,
and they join §3.8 point 2's discipline: the example apps of the four are
part of the same bump-rehearsal suite as the five variants' examples, so
a nightly-pin bump is rehearsed across all nine, and the combined
breakage list is the bump's true cost. As with §3.7's slice, this
programme is recorded here, not enqueued.
