# Cross-links: automation, Cloudflare pipes and DNS

<!-- id: lab-cross-links since: pre-release -->

Three topics sit on the fleet's border, with their primary coverage
owned by the Lab Book — the book of the Code Book + Lab Book
programme (#13905), covering the physical lab, the cluster programme
and the network around both, published at
[developing-today.github.io/code/lab/](https://developing-today.github.io/code/lab/).
This chapter does not duplicate that coverage. It states, for each
topic, what the fleet side is and where the primary coverage lives,
linking only targets that exist — checked against the repo on
2026-11-23.

A note on timing, because it changed mid-lane: the Lab Book's part 1
landed while this chapter was being written (PR #13968, merged
2026-11-23 19:25 CST), with sources at `pkgs/lab/book/`. Part 1
covers the site, the hardware, the cluster programme, the network
switches, the ESP32 line and the roadmap. The three topics below map
onto it unevenly, and this chapter says which is which: automation
has real chapters to link; the pipes are covered inside other
chapters, with no dedicated chapter yet; DNS has no Lab Book chapter
yet, and its owning programme is named instead of a link invented.
The Code Book's summary chapter,
[The Lab](https://developing-today.github.io/code/code/systems/lab.html),
remains the one-page overview of the whole lab programme.

## Automation and automated scale-up / scale-down

**Primary coverage: the Lab Book — landed in part 1.** The lab's
automation story is the cluster programme: Talos Linux bootstrapped
declaratively with TALM, running Cozystack with FluxCD, on the
basement machines of the cluster epic #1817 and the bootstrap epic
#1086. Its scale behaviours are the Lab Book's own chapters:

- [Boot and auto-join](https://developing-today.github.io/code/lab/cluster/boot-and-auto-join.html)
  — merged priority item 5: all servers online, auto-connecting and
  presenting to the cluster on boot; no hand-built machine that just
  exists forever.
- [Talos and Cozystack](https://developing-today.github.io/code/lab/cluster/talos-and-cozystack.html)
  and [The cluster programme](https://developing-today.github.io/code/lab/cluster/the-cluster-programme.html)
  — the platform the machines join, and the programme state around
  it, including "containerised build/compile in the cluster" from
  #13905's seed list.

The state those chapters record is the honest one: as of the Lab
Book's latest status record (2026-10-27) nothing in the current
cluster programme has been built yet, by design — the machines have
no operating system installed, and the build target is the 3–5 best
machines first, gated on basement power (#1104, #1105).

The fleet side of scale is a different mechanism, and it is covered
here, not there: the fleet scales *work*, not machines. Placement
decides where a build runs ([Placement and remote
dispatch](placement.md)); the central compile coordinator is
designed to admit, queue and drain that work against global caps and
budgets ([The central compile coordinator](compile-coordinator.md));
and temporary capacity is borrowed as short-lived Actions runners
that scale to zero when their lease ends ([Temporary builds and
custom connections](temporary-builds.md)).

## The pipes into Cloudflare and externally

**Primary coverage: split, and stated as such.** The fleet's own
doors and tunnels are covered in this book and the PM book, because
they are fleet machinery: the Commander door
([The Commander door: Desktop Commander over MCP](commander.md)),
the bridge, the hosted edition's doors
([The hosted edition: fleet-hosted](fleet-hosted.md)), the exec
door's tunnel ingress
([Temporary builds and custom connections](temporary-builds.md)) and
the queue watchdog that probes one of those pipes from outside
([The queue watchdog: queue-transport-alert](queue-transport-alert.md)).

The lab-side half sits inside the Lab Book's part 1 rather than in a
chapter of its own: [The cluster
programme](https://developing-today.github.io/code/lab/cluster/the-cluster-programme.html)
carries the first lab Cloudflare Tunnel (live 2026-11-06 on the
laptop, seed ingress `lab.security.cab`, #6455) as the programme's
one live artefact, lists the Cloudflare tunnel among the day-one
platform services (epic #1088), and holds the egress section — the
strategy for the 2 Gbps home fiber, designed in #5030 (open):
tunnel or shunt bulk traffic to cheap-or-free-egress services,
compress aggressively. [Switches](https://developing-today.github.io/code/lab/network/switches.html)
covers the physical network the pipes land on. No dedicated
pipes chapter exists in either book yet. The estate-wide half —
Turso and Cloudflare across databases, edge, gateway and DNS — is
epic #8987 (open), with its design record in #8870 and the follow-on
record #13912 (both open); until that programme lands its coverage,
those trackers are the record and this section is the map to them.

## DNS management

**Primary coverage: the Lab Book programme (#13905) for the estate
view — no chapter yet; the working machinery is in this repo today.**
The Lab Book's part 1 SUMMARY (checked 2026-11-23) has no DNS
chapter, and this section names the topic and its owner rather than
linking a page that does not exist. DNS is managed as code under
`infra/dns/`: a Terraform configuration driven by `dns_config.yaml`,
with its apply scripts beside it. The standing items around it are
open and named: the dns-registry work — a recurring DNS backup job,
renewal and cost tracking, and a dashboard view of the domain
register (#902); the registrar and DNS classification for Drew's
domains, with Porkbun as the default layer (#8808); and an apply
window with a recorded gate — clearing a phantom state entry and
deciding nine unmanaged developing-today.com records needs Drew's
go-ahead (#4989). The estate view of DNS also sits inside the Turso
+ Cloudflare epic #8987 above. The fleet book's own DNS surface is
deliberately nil: fleet consumes names (the security.cab doors, the
broker's public route) and manages none of them, so this pointer is
the whole fleet chapter on the subject.

## Sources

Programme #13905 (Code Book + Lab Book programme; deliverables and
seed items); the Lab Book, `pkgs/lab/book/` (part 1, PR #13968,
merged 2026-11-23) and its deploying chapter (published under
`/lab/`); the Code Book, `pkgs/code-book/book/systems/lab.md`;
cluster epics #1817, #1086 and #1088; lab tunnel #6455, #6456,
#8933; egress design #5030; Turso + Cloudflare epic #8987 with #8870
and #13912; `infra/dns/`; DNS items #902, #8808, #4989.
