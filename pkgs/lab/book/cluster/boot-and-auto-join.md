# Boot and auto-join

Merged priority list item 5: **all servers online,
auto-connecting and presenting to the cluster on boot.** Every
server powers on, joins the network and presents itself to the
cluster declaratively — no hand-built machine that "just exists
forever". This chapter records the ambition, the mechanism that
would deliver it, and the current state, which is the deliberate
opposite of a running system.

## The ambition

Drew's seed, as filed on
[#13905](https://github.com/developing-today/code/issues/13905):
"servers auto-connecting and presenting to the cluster on boot".
In the merged priority list it sits directly after "switches
integrated and racked" (item 4) and before "Talos cluster live …
on Cozystack" (item 6) — the cluster is defined over machines
that present themselves, and hand-installing the first nodes
would create exactly the snowflake state the programme forbids.

- **Type:** capability-set.
- **Area:** lab (primary). Cross-links: fleet (enrolment is the
  same shape as fleet agent enrolment); ID (the hosted-worlds
  deployment pack targets these machines — see the
  [High-level roadmap](../roadmap/high-level-roadmap.md)).
- **Most related:** the TALM + Talos bootstrap epic
  [#1086](https://github.com/developing-today/code/issues/1086),
  named as parent of the cluster config in `infra/cluster/README.md`.

## Current state: no OS, by design

**Not started.** The record, stated without softening:

- No OS is installed on any basement machine — deliberately, per
  the 2026-10-27 status record. The absence is the design: "no
  hand-built state that just existed forever".
- Every CPU boots and all RAM works; nothing is load-tested.
- Basement power capacity is a named blocker (see
  [The Osseo site](../site/the-osseo-site.md)); the build target
  is the 3–5 best, most identical machines first.
- Timing: no timing in the record. The hardware state (no OS,
  power blocker, 3–5 first) is as recorded 2026-10-20/21 in the
  hardware register; no reprioritisation is recorded since.

## The mechanism that would deliver it

The mechanism of record is the TALM flow proven in
[Talos and Cozystack](talos-and-cozystack.md):

1. A node boots from the Talos image into maintenance mode.
2. `talm template -i` harvests its real hardware into its per-node
   file in git.
3. `talm apply` lays the node's strategic merge patch — disks
   matched by identity, never by description — over the shared
   cluster template.
4. The node joins, or rejoins, the cluster from config alone. The
   #1812 runbook's wipe-and-rebuild path is exactly this: "the
   node rejoins the cluster from git alone".

The historical test cluster additionally used PXE (an iPXE script
from factory.talos.dev) as a boot path — proven then, retired
now, recorded in `infra/talos/01b.us/README.md`.

Power-return behaviour is separately ticketed as part of epic
[#1090](https://github.com/developing-today/code/issues/1090): a
dependency-sequenced power-return boot order, documented and
exercised once. Planned.

## The gap this chapter states plainly

**There is no dedicated implementation issue for auto-join.**
The census searched for one and found none: the ambition lives in
the #13905 seed, in merged-list item 5, and in the TALM mechanism
that would deliver it — not in a ticket of its own. This book
does not invent one, does not borrow #1086's number as if it were
the auto-join issue, and records the absence here so the
programme can file the issue deliberately. Until then, #1086 is
the closest parent and the TALM flow is the mechanism of record.
