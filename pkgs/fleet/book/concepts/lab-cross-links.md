# Cross-links: automation, Cloudflare pipes and DNS

<!-- id: lab-cross-links since: pre-release -->

Three topics sit on the fleet's border, with their primary coverage
owned by the Lab Book — the book of the Code Book + Lab Book
programme (#13905), covering the physical lab, the cluster programme
and the network around both. This chapter does not duplicate that
coverage. It states, for each topic, what the fleet side is, where
the primary coverage lives, and what state that coverage is in —
checked against the repo and the issue record on 2026-11-23. Where a
target does not exist yet, this chapter names the topic and its
owning programme instead of inventing a link.

The one Lab target that exists today is the Code Book's chapter
[The Lab](https://developing-today.github.io/code/code/systems/lab.html),
which summarises the lab, its cluster programme and its trackers. The
Lab Book itself has **not landed yet**: no Lab Book sources exist in
this repo and no deploy staging carries it (checked 2026-11-23). Its
sources and chapter map are being settled by the Lab Book lane under
#13905. The fleet-side halves of all three topics are covered in this
book and linked below; the lab-side halves belong to that programme.

## Automation and automated scale-up / scale-down

**Primary coverage: the Lab Book (programme #13905) — not yet
landed.** The lab's automation story is the cluster programme: Talos
Linux bootstrapped declaratively with TALM, running Cozystack with
FluxCD, on the basement machines of the cluster epic #1817 and the
bootstrap epic #1086. The programme's own seed list in #13905 names
the scale behaviours directly: "Talos cluster live and accepting
data on Cozystack", "servers auto-connecting and presenting to the
cluster on boot", and "containerised build/compile in the cluster".
The Code Book's Lab chapter records the honest state beneath those
items: as of its latest status record (2026-10-27) nothing in the
current cluster programme has been built yet, by design — the
machines have no operating system installed, and the build target is
the 3–5 best machines first, gated on basement power (#1104, #1105).

The fleet side of scale is a different mechanism, and it is covered
here, not there: the fleet scales _work_, not machines. Placement
decides where a build runs ([Placement and remote
dispatch](placement.md)); the central compile coordinator is
designed to admit, queue and drain that work against global caps and
budgets ([The central compile coordinator](compile-coordinator.md));
and temporary capacity is borrowed as short-lived Actions runners
that scale to zero when their lease ends ([Temporary builds and
custom connections](temporary-builds.md)). When the Lab Book lands,
its automation chapters are the primary home for cluster scale-up
and scale-down; this section is the pointer, and the fleet chapters
above stay the primary home for dispatch-side scaling.

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

The lab-side and estate-wide halves belong to the Lab Book programme
and the Turso + Cloudflare integration epic: the first lab
Cloudflare Tunnel, live 2026-11-06 on the laptop with the seed
ingress `lab.security.cab` (#6455, closed; connector and watchdog
#6456, #8933, closed); the cluster egress strategy for the 2 Gbps
home fiber — budgets, tunnel topology, failure matrix — designed in
#5030 (open); and the estate-wide integration of Turso and
Cloudflare across databases, edge, gateway and DNS, epic #8987
(open), with its design record in #8870 and the follow-on record
#13912 (both open). No single landed chapter covers "the pipes" as
one subject yet; until the Lab Book or the #8987 programme lands one,
the trackers above are the record, and this section is the map to
them.

## DNS management

**Primary coverage: the Lab Book (programme #13905) for the estate
view — not yet landed; the working machinery is in this repo today.**
DNS is managed as code under `infra/dns/`: a Terraform configuration
driven by `dns_config.yaml`, with its apply scripts beside it. The
standing items around it are open and named: the dns-registry work —
a recurring DNS backup job, renewal and cost tracking, and a
dashboard view of the domain register (#902); the registrar and DNS
classification for Drew's domains, with Porkbun as the default layer
(#8808); and an apply window with a recorded gate — clearing a
phantom state entry and deciding nine unmanaged
developing-today.com records needs Drew's go-ahead (#4989). The
estate view of DNS also sits inside the Turso + Cloudflare epic
#8987 above. The fleet book's own DNS surface is deliberately nil:
fleet consumes names (the security.cab doors, the broker's public
route) and manages none of them, so this pointer is the whole fleet
chapter on the subject.

## Sources

Programme #13905 (Code Book + Lab Book programme; deliverables and
seed items); the Code Book, `pkgs/code-book/book/systems/lab.md` and
its SUMMARY (checked 2026-11-23: no Lab Book sources or deploy
staging in the repo); cluster epics #1817 and #1086; lab tunnel
#6455, #6456, #8933; egress design #5030; Turso + Cloudflare epic
#8987 with #8870 and #13912; `infra/dns/`; DNS items #902, #8808,
#4989.
