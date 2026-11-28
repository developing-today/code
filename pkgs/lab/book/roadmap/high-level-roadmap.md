# High-level roadmap

This chapter is the Lab Book's section of the project's merged
priority list (census B, gathered 2026-11-23/24 under
[#13905](https://github.com/developing-today/code/issues/13905)).
The ordering logic of the whole list, stated once there:
execution substrate first, then the physical substrate, then the
data substrate, then search intelligence, then the ID platform
build-out, then the gateway/PM/fork surfaces, then the record
itself. Within a tier, dependency order wins; ties break on
leverage.

The lab's assignment (§2 of the census): **four primary items —
4, 5, 6 and 24 — in full below, and four cross-links — ◦2, ◦7,
◦17 and ◦30 — as one-liners.** Timing is from §3 of the census.
Where the record has no timing, this chapter says so; no dates
are invented anywhere in it.

## Primary items

### 4. Switches integrated and racked (SEED)

- **Type:** milestone.
- **Description:** The dedicated switches from the repo's
  switching research are installed, racked and integrated, so the
  basement machines share a real fabric instead of ad-hoc links.
- **Area:** lab (primary, sole). Cross-link: fleet (cluster
  networking assumptions).
- **Most related:** the code repo's dedicated switches research —
  `doc/inventory/routing-and-switching.md` and `doc/hardware/`
  (see [Switches](../network/switches.md)). At census B's
  gathering, no single GitHub issue owned this item; the owning
  issue now is
  [#13920](https://github.com/developing-today/code/issues/13920),
  filed under #13905 — complete the inventory, then integrate and
  rack. Per-switch physical tickets:
  [#6073](https://github.com/developing-today/code/issues/6073),
  [#6074](https://github.com/developing-today/code/issues/6074),
  [#6075](https://github.com/developing-today/code/issues/6075);
  fabric epic
  [#1084](https://github.com/developing-today/code/issues/1084);
  NIC/switch verification
  [#1097](https://github.com/developing-today/code/issues/1097).
- **Status today:** **not started (planned).** At census B,
  switch make/model/count was explicitly missing from the
  hardware record until the switching research was folded in; the
  lab census (census C) performed that fold-in — it is the
  [Switches](../network/switches.md) chapter. No OS is installed
  on any basement machine yet, and no switch is recorded as
  racked or carrying cluster traffic.
- **Timing (§3):** no timing in record. The switch inventory
  itself was incomplete (make/model/count missing) — the lab
  census under #13905 is the first dated step.
- **Ordering rationale:** First physical dependency: racking and
  networking precede servers presenting (item 5) and the cluster
  boot (item 6). Cheap, unblocking, and entirely Drew-side /
  logistics-paced.

### 5. All servers online, auto-connecting and presenting to the cluster on boot (SEED)

- **Type:** capability-set.
- **Description:** Every server powers on, joins the network and
  presents itself to the cluster declaratively — no hand-built
  machine that "just exists forever."
- **Area:** lab (primary). Cross-links: fleet (enrolment is the
  same shape as fleet agent enrolment); ID (the hosted-worlds
  deployment pack targets these machines).
- **Most related:** the TALM + Talos bootstrap epic
  [#1086](https://github.com/developing-today/code/issues/1086)
  (named as parent of the cluster config in
  `infra/cluster/README.md`); the ID hosted-worlds P5
  basement-cluster deployment pack
  [#10697](https://github.com/developing-today/code/issues/10697);
  the fleet agent enrolment family around
  [#12236](https://github.com/developing-today/code/issues/12236).
  There is **no dedicated implementation issue** for auto-join
  itself — see [Boot and auto-join](../cluster/boot-and-auto-join.md).
- **Status today:** **not started.** Record: no OS installed on
  the basement fleet (deliberate); every CPU boots, all RAM
  works, nothing load-tested; basement power capacity is a named
  blocker; build target is 3–5 best/most-identical machines
  first.
- **Timing (§3):** no timing in record. The hardware state is as
  recorded 2026-10-20/21 in the hardware register; no
  reprioritisation recorded since.
- **Ordering rationale:** The cluster (item 6) is defined over
  machines that present themselves; hand-installing the first
  nodes would create exactly the snowflake state the programme
  forbids.

### 6. Talos cluster booted live, accepting data on Cozystack (SEED)

- **Type:** milestone.
- **Description:** A Talos Linux cluster bootstrapped
  declaratively with TALM, running Cozystack with Postgres
  (CNPG), RustFS, NATS, FluxCD and DRBD+LINSTOR replication —
  live and accepting data. Hard rule of record: no HashiCorp
  tooling.
- **Area:** lab (primary). Cross-links: fleet (in-cluster builds,
  item 2); ID (hosted worlds deployment target); Atlas (storage
  tiers).
- **Most related:** the TALM + Talos bootstrap epic
  [#1086](https://github.com/developing-today/code/issues/1086);
  `planning/roadmaps/basement-cluster.md` (the current programme
  doc; the v1.9-era test cluster in `doc/talos-addresses.md` is
  torn down, confirmed by Drew 2026-10-26); the ID deployment
  pack [#10697](https://github.com/developing-today/code/issues/10697).
  Programme parent:
  [#1817](https://github.com/developing-today/code/issues/1817).
- **Status today:** **designed / not started.** Design and
  per-node disk config exist (`infra/cluster/`; the TALM flow is
  the one proven piece — see
  [Talos and Cozystack](../cluster/talos-and-cozystack.md));
  nothing is running. The storage design sketch is Drew's, to be
  validated: 15–16× 16 TB HDDs (8 still boxed), at least 3 TB ECC
  RAM, RAID-10/RAID-100 NVMe tiers, 3-machine DRBD replication.
- **Timing (§3):** the v1.9-era Talos test cluster was torn down
  (confirmed by Drew 2026-10-26) — a recorded miss/reset: the
  current programme (`basement-cluster.md`) restarts from
  declarative bootstrap. No live date in record.
- **Ordering rationale:** The physical keystone: in-cluster
  builds (item 2), self-hosted ID servers (item 17) and the local
  leg of the backup mesh (item 7) all presume it. Ranked below
  its own prerequisites (items 4–5) by dependency, above
  everything that consumes it by leverage.

### 24. ESP32 hardware platform line — ID on microcontrollers

- **Type:** capability-set.
- **Description:** ESP32-family boards as an ID hardware shell —
  sensors and LoRa connected to the ID project — a lab chapter
  today, its own line long-term.
- **Area:** lab (primary). Cross-link: ID (the hardware shell
  crate `id-hw-shell` and the shells programme, item 22 of the
  merged list).
- **Most related:**
  [#5783 — ESP32 hardware platform line](https://github.com/developing-today/code/issues/5783)
  (open, design; already tasked by a completed first pass);
  per-device verification docs in `developing-today/hardware-doc`
  (mostly ESP32 boards today — exact part names are verified
  there, not guessed). Wing epic:
  [#1093](https://github.com/developing-today/code/issues/1093).
  Full chapter: [ESP32](../microcontrollers/esp32.md).
- **Status today:** **designed / side-line.** Recorded in the
  hardware census as "a side project of a side project for now —
  noted, not scheduled."
- **Timing (§3):** no timing in record; explicitly "noted, not
  scheduled" in the hardware record.
- **Ordering rationale:** Genuinely long-term: it inherits the
  shell contract (item 22) and the lab's physical readiness
  (items 4–6), and blocks nothing.

## Cross-links

One line each, by the census's completeness rule — the full text
of each item lives in its primary book.

- **◦2 — Containerised build/compile systems (fleet primary):**
  the in-cluster half runs on item 6 and is not started — it
  waits on the cluster; the Actions/compile-coordinator half is
  designed and in flight in the fleet book's chapter:
  [Containerised builds — fleet book](https://developing-today.github.io/code/fleet/).
- **◦7 — Backups + storage automated, one coherent complete
  backup set (Atlas primary):** the local leg runs on cluster
  storage — the DRBD/LINSTOR tiers of
  [Storage: LINSTOR and DRBD](../cluster/storage-linstor-and-drbd.md);
  the mesh itself is the Atlas book's chapter.
- **◦17 — Tunnels to spin up ID servers dynamically (ID
  primary):** the basement-cluster deployment pack for hosted ID
  worlds is
  [#10697](https://github.com/developing-today/code/issues/10697)
  (P5 of the hosted-worlds chain) and runs on item 6; the chain
  itself is the ID book's chapter.
- **◦30 — The books and Spaces themselves (PM primary):** this
  book is that programme line's lab deliverable — the Lab Book
  plus the Lab Space plus the AliExpress inventory issue
  [#13910](https://github.com/developing-today/code/issues/13910),
  coordinated on
  [#13905](https://github.com/developing-today/code/issues/13905).
