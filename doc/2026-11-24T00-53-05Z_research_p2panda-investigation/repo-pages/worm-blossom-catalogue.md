# worm-blossom catalogue — condensed digest

This page condenses a 101 KB week-by-week catalogue of the worm-blossom
collective's public diary and site. The full catalogue stays in the
investigation workspace (`~/workspace/p2panda-investigation/research/worm-blossom-catalogue.md`);
what follows is its structure and headline findings, in paraphrase —
the diary is a public source, but this bundle paraphrases rather than
quoting it. The analytical Willow/Meadowcap findings drawn from the same
material live in [Willow & Meadowcap](willow-meadowcap.md); the targeted
question list the team worked from is summarised in that page's sources.

## What the catalogue is

A complete read of the public record of **worm-blossom** — the
three-person collective behind the Willow protocol and Meadowcap:
**Aljoscha Meyer**, **Sammy Gwilym**, and **Emmy Walker** (who joined as
the third member in autumn 2026 and works on Pillow). The catalogue
covers **59 weekly diary updates**, counted from the site's own archive
index: 2025 weeks 41–52 (12 updates), 2026 Part 1 weeks 01–26 (26), and
2026 Part 2 weeks 27–47 (21) — plus the site's two index pages (All
Music; All Comics, with 43 comic entries — the collective publishes art
alongside protocols, and the diary mixes both) and its project roster:
**20 maintained projects and 3 independent implementations** by others
(Ourcana's Dart Willow implementation, Freckles' willow-ts, and a Zig
implementation of the Bab component).

The catalogue's value to this programme is due diligence depth: the
Willow/Meadowcap recommendation in the synthesis rests partly on this
team's engineering honesty, and the diary — unusually candid for a
protocol project — is where that honesty is documented.

## Headline findings

**Pillow is the bridge to our stack.** Announced in the 2026 week-35
update, Pillow is a **p2panda × worm-blossom joint grant project**:
p2panda applications storing data via highly-available Willow peers,
with **Meadowcap permissions**, eviction of stale data, and sync via the
Willow Transfer Protocol (WTP). The two halves of our recommended data
plane are already being integrated by their own authors, with grant
money behind the integration.

**The 2025 week-52 post-mortem resets the sync story.** The collective
published a maximalism post-mortem admitting that their **Confidential
Sync** design had become too complex, and demoted it; the doctrine that
replaced it is **pull-based sync**, with WTP plus a capability-scoped
Willow server named as the future direction. Practical consequence for
anyone citing Willow: **Confidential Sync is not Willow's sync story**
and should never be presented as such; the live story is WTP + Meadowcap
scoping.

**WTP is real but still settling.** Across weeks 44, 32, and 35–36 the
diary shows the transfer protocol's spec published _without_ a wire
encoding, message types still being merged and symmetrised, support for
both stateful and stateless operation, resumable transfers verified
against the Bab test tooling — and, as of week 47, encodings described
as close to done. WTP should be treated as a late-draft protocol:
adoptable via the authors' own stack, risky to implement independently
from the spec today.

**AreaSlices dissolve a storage objection (2026 week 45).** Work on
Amparore/AreaSlices showed that arbitrary Willow _Area_ queries can be
answered with denormalised 1D/2D indexes — which largely dissolves the
"3D storage" objection to implementing a Willow store over SQL/libsql.
This finding feeds directly into the estate's storage planning: a
Willow-shaped store does not require an exotic index structure.

**Two honesty tests, passed in public (weeks 46, 37, and 26→28).**
First, the diary names a real tension between **prefix-pruning** (an
efficiency technique) and **Meadowcap scoping** (a confidentiality
technique): mitigated by peer-discretionary disclosure, explicitly _not_
solved. Second, their **test-vector programme caught a serious missing
check in the capability verification implementation** — found via an
external implementer, disclosed in the diary. Third, the WILLIAM3 arc:
in June 2026 the reference `bab_rs` tool produced **incorrect digests**,
found by the Zig implementers; version 0.8.0 restored confidence by
diffing against BLAKE3 modulo documented spec deltas. There has been
**no independent audit** — the pattern to note is not the bugs but that
cross-implementation testing found them and the diary reported them.

**Rocket is the hard composition constraint (weeks 33 and 42).** In the
same period the collective stripped generics from the codebase
(willow25 0.7.0, roughly −15k lines; the `willow_data_model` and
`meadowcap` crates were deprecated in the process), the diary records
the **Rocket failure**: the whole stack is built on tokio's
`current_thread` runtime only, with frugal async and **no `Send`
bounds** — a hard constraint when composing with other async Rust (the
authors flag it as an open worry themselves). Any estate integration
must budget for running the Willow stack on its own thread/runtime
island.

**Ourcana is the only production deployment (week 23 interview).**
Roughly 9–10k users, on an **independent Dart implementation** that is
bit-compatible over the wire — the strongest available evidence that the
specs are implementable by third parties. Production feedback changed
the Drop Format spec itself (streaming "dripdrop"), which is what a
healthy spec/implementation loop looks like.

## The surprises section (the catalogue's Part 3)

- **The diary itself is the surprise**: a weekly, named, self-critical
  engineering record is an unusually strong due-diligence source —
  stronger evidence about team quality than any roadmap document.
- **"Final" doesn't mean final.** The Data Model and Meadowcap had been
  promoted to "Final" on implementation evidence alone; in November 2026
  the collective **changed their own rule to require real-world usage**,
  retroactively weakening their own labels. Cite their maturity claims
  with the new rule, not the old labels.
- **Encryption is the thinnest layer.** Drop encryption was _removed_
  from the Drop Format spec and left to applications; Willow'25 at-rest
  encryption has generic designs but, per week 36, no specific
  instantiation yet. Anyone assuming "Willow encrypts the data" is
  ahead of the specs.
- **The ecosystem is real but small and personal**: grant-to-grant
  funding (the Bab grant ended mid-2026; Pillow is the current vehicle),
  a team of three, independent implementers who are individuals. This is
  a strength for coherence and a risk for continuity — the same
  stewardship lens the IPFS lane applies, at the opposite scale.
- **They refuse LLM-generated contributions** (policy formalised in week
  46). Directly relevant to this estate: upstreaming patches to
  worm-blossom projects from agent lanes is not available as a strategy;
  integration work must be framed as issues, specs, and test vectors, or
  done on our side of the boundary.
- Two catalogue-hygiene notes: **"sneaker-world" is not a project** —
  it is a jocular dateline in the diary, and should not appear in any
  project list; and on **GPUI** there is no worm-blossom position — the
  only trace is Sammy sharing zortax's "A Critique of GPUI" in week 47
  with a brief Elm-architecture comment, which is a link-share, not a
  stance.
