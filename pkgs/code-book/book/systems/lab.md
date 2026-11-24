# The Lab

The Lab is the project's physical side: Drew's basement in Osseo, Wisconsin,
the machines in it, the home network around them, and the cluster programme
that will turn the machines into the project's own compute and storage
substrate. Its census (Census C, 2026-11-23/24) opens with the headline this
chapter repeats without softening: as of the latest status record
(2026-10-27), nothing in the current cluster programme has been built yet —
and that is by design, not by drift. The machines have no operating system
installed, deliberately: no hand-built machine that just exists forever.
What exists is thorough research, a proven local bootstrap flow, one live
tunnel, and a large ticketed programme.

## What is physically there

- About a dozen Threadripper machines, identical in motherboard, CPU and
  specifications except for their SSDs; the named core board is the ASUS Pro
  WS WRX80E-SAGE. Every CPU boots and all RAM works; nothing has been
  load-tested. Some Dell machines and several spare PCs and laptops stand
  alongside, plus 2–3 mini PCs named for the cluster interconnect (#6077).
- Power is the named blocker: basement power capacity blocks the full fleet,
  and the build target is the 3–5 best, most identical machines first. The
  electrician brief is complete as a document (four dedicated 20-amp
  circuits); engaging an electrician (#1104) and installing the circuits
  (#1105) are open.
- Storage, as Drew's sketch of record: 15–16 drives of 16 TB (8 still boxed),
  at least 3 TB of ECC RAM across the fleet, NVMe tiers, and 3-machine
  replication. The sketch is Drew's, to be validated — the Lab Book carries
  it as a sketch, not as a specification.
- The home network: 2 Gbps symmetric Brightspeed fiber, a TP-Link Deco
  mesh in router mode, and one live lab artefact — the first lab Cloudflare
  Tunnel, live 2026-11-06 on the laptop with the seed ingress
  `lab.security.cab` (#6455, closed). It runs on the laptop, not on any
  cluster machine.

## The cluster programme

The programme is Talos Linux bootstrapped declaratively with TALM, running
Cozystack with Postgres (CloudNativePG), RustFS, NATS and FluxCD, and
storage replicated with DRBD and LINSTOR. The hard rule of record: no
HashiCorp tooling. Its state, component by component (Census C):

- **TALM per-node disk configuration: proven as design and local flow.**
  Issue #1812 is closed via PR #5052: one TALM node file per node in git,
  disks matched by identity (serial or WWID), never by size or model alone.
  The local container flow was exercised end to end; bare-metal proof waits
  on the hardware inventory (#1810).
- **Talos and Cozystack: planned.** No node is installed until the TALM
  bootstrap flow is proven. The earlier v1.9-era test cluster is historical
  reference only — torn down, confirmed by Drew 2026-10-26.
- **Switches: researched, not integrated.** The dedicated-switches research
  lives in `doc/inventory/routing-and-switching.md` and `doc/hardware/`.
  At census no single GitHub issue owned the switches item — the Lab
  programme on #13905 was its first tracker. History note (2026-11-23):
  the census found this item ownerless and the programme filed the
  owning issue that day —
  [#13920](https://github.com/developing-today/code/issues/13920) —
  which now owns it. The inventory gap is real and stays: the
  inventory of record enriches 21 devices with make and model, but the
  record still lacks location and state per device; #13920's first
  slice owns completing it, and Census C's findings are recorded on
  #13920.
- **ESP32 microcontrollers: a side line.** Per-device verification lives in
  the sibling `developing-today/hardware-doc` repository, which is mostly
  ESP32 boards today. The programme issue is #5783 (open, design); the
  hardware census records the line as a side project of a side project —
  noted, not scheduled.

## Where its detail lives

- Book: the Lab Book, built under this same programme and published at
  [developing-today.github.io/code/lab/](https://developing-today.github.io/code/lab/) —
  sources and chapter map are settled by the Lab Book lane under #13905.
- Programme trackers: [#13905](https://github.com/developing-today/code/issues/13905)
  (this programme), [#1817](https://github.com/developing-today/code/issues/1817)
  (basement Talos/Cozystack cluster epic), [#1086](https://github.com/developing-today/code/issues/1086)
  (TALM and Talos bootstrap epic), [#6066](https://github.com/developing-today/code/issues/6066)
  (physical bring-up parent).
- Merged-list items: 4, 5, 6 and 24 are lab-primary; the lab also carries
  cross-links to items 2, 7, 17 and 30.
