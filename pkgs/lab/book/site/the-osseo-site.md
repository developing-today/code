# The Osseo site

The lab is one physical place: the basement of Drew's house in
Osseo, Wisconsin (America/Chicago), plus the home network the house
already runs. Drew plans to stay in the house long-term — 10+
years, possibly 40 — and home Ethernet hardwiring is a standing
project (`doc/platform/hardware.md` in the code repository, sourced
from the idea register of 2026-10-20).

## The basement fleet, as recorded

About a dozen Threadripper machines form the core fleet. The record
(idea register 2026-10-20; roadmap `planning/roadmaps/basement-cluster.md`;
status record 2026-10-27) states:

- The core boxes are identical in motherboard, CPU and
  specifications, except for their SSDs.
- One board is named: the ASUS Pro WS WRX80E-SAGE ("WRX80 Sage"),
  which received a firmware update in the 2026-10-20 pass. The
  sources do not say whether every core box is this board or only
  one is.
- A separate group of Dell machines was picked up alongside; no
  firmware updates could be found for them, and their BIOS received
  a best effort.
- Every CPU at least boots and all RAM works. Nothing has been
  load-tested.
- No operating system is installed on any of them — deliberately.
  The programme's rule is that there will be no hand-built state
  that just existed forever; machines will be installed from
  configuration when the cluster programme reaches them. See
  [Boot and auto-join](../cluster/boot-and-auto-join.md).
- The core machines have enterprise BMC/IPMI remote management
  (built-in enterprise KVM). Versions and every setting are still
  to be audited — that audit is epic
  [#1081](https://github.com/developing-today/code/issues/1081).
- Two to three mini PCs are planned to join the cluster
  interconnect alongside the dozen machines
  ([#6077](https://github.com/developing-today/code/issues/6077)).
- Several older PCs and laptops can be hooked up quickly — decent
  when bought, shelved for about two years, at least one with an
  integrated GPU — alongside a desktop tower, an older Framework
  laptop and a gaming PC. Their candidate roles are in
  [The cluster programme](../cluster/the-cluster-programme.md).
  The source for this group is an addendum of 2026-10-21 marked
  unverified dictation, and this book keeps that marking.

Status of the fleet as a whole: **planned**. The machines exist;
nothing is installed on them.

## Power

Power is the blocker of record for the full fleet: basement power
capacity and general mess stand in the way, and the target is 3–5
machines first.

The electrician brief exists and is complete as a document
(`planning/roadmaps/basement-cluster-electrician-brief.md`): four
dedicated 20A circuits, each on its own breaker, 12 AWG copper,
duplex receptacles at the rack location, labelled "CLUSTER 1"
through "CLUSTER 4". Its load maths uses 600 W sustained per
machine — a planning assumption in the brief itself, to be replaced
by measured values. An earlier UPS-breaker drop-off idea is
explicitly not wanted.

Ticket state:

- [#1103](https://github.com/developing-today/code/issues/1103) —
  circuit spec and electrician brief: **closed**.
- [#1104](https://github.com/developing-today/code/issues/1104) —
  engage an electrician (quotes and schedule): open.
- [#1105](https://github.com/developing-today/code/issues/1105) —
  install and verify the dedicated 20A circuits: open.
- Epic: [#1083](https://github.com/developing-today/code/issues/1083)
  — basement power and space.

The census found no record of circuits actually installed, a UPS
in place, or measured per-machine draw. That is gap 6 in
[Gaps and open questions](../plans/gaps-and-open-questions.md).

## Racking and physical layout

- [#1106](https://github.com/developing-today/code/issues/1106) —
  basement cleanup and rack layout for the first nodes: open.
- [#6070](https://github.com/developing-today/code/issues/6070) —
  "another 4 each of racking (as dictated)": open. The recorded
  interpretation is "four more rack units / items to rack"; the
  ticket itself asks that what the four turned out to be be
  recorded on the ticket. The dictation is not resolved in the
  record, and this book does not resolve it.
- [#6071](https://github.com/developing-today/code/issues/6071) —
  install 3 more DX010 units: open. See [Switches](../network/switches.md).

No record was found of a rack actually populated, rails installed,
or a completed layout. The physical bring-up sequence is ticketed
under parent
[#6066](https://github.com/developing-today/code/issues/6066)
(filed 2026-11-04): racking, then Dell switches ×3, then edge
routing and mesh Wi-Fi, then interconnect, then cluster bring-up.
All of it is planned; none of its checkboxes were checked in the
bodies the census read.

## The home network as it stands today

| Fact                                                                                                                                                                                                                                                                                       | Status                                                                                     |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------ |
| WAN: 2 Gbps symmetric fiber from Brightspeed — the only fiber provider in this part of Osseo. A second line would make 4 Gbps. Cable is 100 Mbps and pricier (a possible redundancy pickup). TriCounty Communications fiber may reach the address later; then both fibers would be bought. | Proven, as the standing service of record                                                  |
| ISP customer-premises equipment in the switching inventory: Calix GP1101X GigaPoint ONT, XGS-PON 10G/10G, one 10GBASE-T port                                                                                                                                                               | Researched-only (inventory record; no install state stated)                                |
| Current LAN: Brightspeed fiber into a TP-Link Deco BE11000 in router mode                                                                                                                                                                                                                  | Proven (memory of record)                                                                  |
| Home IPv6 is broken in practice: IPv6 payload transfers to Cloudflare/npmjs stall at 0 bytes; PMTUD black hole is the lead suspect. IPv6 is ULA-only; the standing directive is to prefer IPv4 everywhere until fixed ([#1280](https://github.com/developing-today/code/issues/1280))      | In progress — a workaround landed for the laptop (PR #1286); the root fix is open on #1280 |
| Drew's networking priority, ruled 2026-11-05: router and Wi-Fi first                                                                                                                                                                                                                       | Standing direction                                                                         |
| Edge routing reconfigure and rewire ([#6067](https://github.com/developing-today/code/issues/6067)) — sequenced after the Dell switches are racked; the edge router's config is to be stored in the code repository                                                                        | Planned                                                                                    |
| Mesh Wi-Fi rewire ([#6068](https://github.com/developing-today/code/issues/6068))                                                                                                                                                                                                          | Planned                                                                                    |

### The one live artefact

The lab programme's first Cloudflare Tunnel went live on
2026-11-06: a connector on the laptop (`amd`), seed ingress
`lab.security.cab`, NixOS service `services.lab-tunnel`
([#6455](https://github.com/developing-today/code/issues/6455),
closed). It is **proven** — and it is also the one live network
artefact of the whole lab programme. Note where it runs: on the
laptop, not on any cluster machine.

## What is not at the site yet

No running Kubernetes cluster. No installed switch fabric for the
cluster. No racked cluster nodes. No cluster storage pools. The
v1.9-era Talos test cluster (VIP 10.10.0.42, controllers a1/a2/b1,
workers a3/a4/b2–b6, recorded in `infra/talos/01b.us/`) is
historical reference only — torn down, confirmed by Drew
2026-10-26. See [Talos and Cozystack](../cluster/talos-and-cozystack.md).
