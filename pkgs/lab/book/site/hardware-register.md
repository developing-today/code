# Hardware register

This chapter records the lab's hardware exactly as far as the
record supports it — and names, just as exactly, where the record
stops. The register's figures come from Drew's idea register of
2026-10-20 and the records built on it. Where two figures exist,
both are given.

## The core fleet

| Item                        | Record                                                                                                                                                                                                                                                                                                                                                                                                             |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Threadripper machines       | About 12 ("a dozen"). Identical motherboard, CPU and specifications except SSDs; one named board is the ASUS Pro WS WRX80E-SAGE. Enterprise BMC/IPMI on the core machines. High-speed NICs whose exact specification is still to be verified — "at least 200 Gb ports" was dictated and remains unverified (see the gap list). Every machine has a consumer NVMe boot drive; the best are Samsung 990 Pro.         |
| RAM                         | At least 3 TB of DDR4 server ECC RAM across the fleet. All of it works; none of it is load-tested.                                                                                                                                                                                                                                                                                                                 |
| Hard drives                 | 15–16 × 16 TB helium Western Digital server pulls (Drew also said "at least 12" once; the register records both figures). Bought before storage prices spiked. Eight are still in the box, uninstalled, with no SMART or baseline data captured for any of the HDDs.                                                                                                                                               |
| NVMe and SSDs               | Every machine has some consumer NVMe boot drive. Best tier: prosumer Samsung 990 Pro. Mid tier: 980s, smaller 990s, other good consumer SSDs. Per-machine SSDs are the one specification that differs between core boxes.                                                                                                                                                                                          |
| NVMe firmware/SMART archive | A backup hard drive holds SMART data and manufacturer firmware data for every NVMe (not the HDDs). Firmware updates were run wherever findable. Proven as recorded, 2026-10-20.                                                                                                                                                                                                                                    |
| Firmware state              | WRX80E-SAGE updated. Dell machines: no updates found, BIOS best effort. Every CPU boots, all RAM works. The full audit is epic [#1081](https://github.com/developing-today/code/issues/1081), with [#1099](https://github.com/developing-today/code/issues/1099)–[#1102](https://github.com/developing-today/code/issues/1102) (golden baseline, bring-to-baseline, routine KVM screenshot verification) all open. |

## The rest of the site hardware

| Machine or group           | Record                                                                                                                                                                                                                                                                                                                   |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Dell machines              | Part of the basement fleet. Count not recorded; no further specification in the sources read; no firmware updates found for them. A gap.                                                                                                                                                                                 |
| Mini PCs                   | 2–3, planned as cluster interconnect members ([#6077](https://github.com/developing-today/code/issues/6077)). No specification in the record. A gap.                                                                                                                                                                     |
| Spare PCs and laptops      | Several, plus a desktop tower, an older Framework laptop and a gaming PC. Candidate roles: compile/test runners, iGPU remote-desktop hosts, Talos nodes, quorum witnesses — see [The cluster programme](../cluster/the-cluster-programme.md). Source marked unverified dictation in the register addendum of 2026-10-21. |
| Second NixOS host, `nixos` | A Framework 13-inch 12th-gen Intel machine with a 2 TB NVMe layout and desktop profile, defined in `nixos/hosts/default.nix`. No lab role recorded. A gap.                                                                                                                                                               |
| PC Engines APU             | A hardware configuration exists (`nixos/hardware-configuration/pcengines/apu/`). No lab role recorded. A gap.                                                                                                                                                                                                            |

Out of scope for the lab, recorded so the boundary is plain: the
work Mac (a Vertex job machine — explicitly not lab hardware) and
Drew's phone.

## GPUs — explicitly not a compute resource

The register lists 3× Intel B50 and 1× RTX 2060 (noted: low hash
rate), plus very old display cards useful only for basic GUI/BIOS
access on a misbehaving box. There are no local model-farm plans.

One framing tension is recorded, not reconciled: a separate closed
research issue,
[#6072](https://github.com/developing-today/code/issues/6072),
covered GPU options after Modal credits and its title records
"local (none owned)" for that capacity question. This book uses the
register's list as the lab's hardware of record and notes the
difference; see gap 10 in
[Gaps and open questions](../plans/gaps-and-open-questions.md).

## The one collected machine record: the laptop `amd`

Exactly one machine in the whole estate has a collected inventory
record: Drew's Framework laptop, `amd`
(`doc/hardware/machines/amd.md`, collected 2026-10-29 by
`machines/collect-inventory.py`):

- Framework Laptop 13, AMD Ryzen 7 7840U (8 cores / 16 threads).
- About 128 GB RAM (132,752,572,416 bytes as collected).
- Samsung SSD 990 PRO 4TB NVMe.
- MediaTek MT7922 Wi-Fi.
- BIOS INSYDE 03.18.
- NixOS host config `amd` in `nixos/hosts/default.nix` (Framework
  13-inch 7040-AMD hardware, 4 TB NVMe disk layout). The laptop is
  Drew's primary workstation, the canonical Atlas home and ingest
  factory, and the machine that runs the lab's Cloudflare Tunnel
  connector — see [The Osseo site](the-osseo-site.md).

The laptop is proven and in daily use. It is also the exception
that proves the rule of the next section.

## The per-machine inventory gap

There are no NixOS definitions for any Threadripper or cluster
machine in `nixos/hosts/` — the two Framework laptops are the only
hosts defined. That is consistent with the design (the cluster
machines will run Talos, not NixOS, and currently run nothing),
but the deeper gap is inventory itself:

- The inventory-record schema and template are built, and the
  design task is closed
  ([#1094](https://github.com/developing-today/code/issues/1094)).
  The schema is in use in `doc/hardware/machines/` — for the
  laptop alone.
- Gathering statistics is still open for the core Threadrippers
  ([#1095](https://github.com/developing-today/code/issues/1095)),
  for the remaining machines plus the boxed-HDD baseline
  ([#1096](https://github.com/developing-today/code/issues/1096)),
  and for ranking and naming the first 3–5 nodes
  ([#1098](https://github.com/developing-today/code/issues/1098)).
- The umbrella is
  [#1810](https://github.com/developing-today/code/issues/1810) —
  hardware inventory and first-node selection, under epic
  [#1080](https://github.com/developing-today/code/issues/1080).

Until those land, every "identical core boxes" claim in this book
rests on Drew's 2026-10-20 dictation. The book says so rather than
letting the claim harden into a specification.

## The completion path: the AliExpress inventory

No per-item prices exist in any source the census read — no
purchase prices for machines, boards, RAM, NVMe, GPUs, NICs or
switches. The only timing note is that the HDDs were bought before
storage prices spiked.

The priced inventory is precisely what
[#13910](https://github.com/developing-today/code/issues/13910) is
meant to create: aggregate the AliExpress purchase history into a
hardware inventory. That issue is open; its access step is pending
Drew and no login was attempted by the programme. When it lands,
this chapter gains the per-item record it currently lacks — and
[Keeping this book current](../development/keeping-this-book-current.md)
makes updating it part of the sync rule.

## A note on the hardware-doc repository

The sibling repository `developing-today/hardware-doc` (symlinked
into the code repository as `doc/hardware`) holds device and
component research decomposed to primary evidence, with every
claim carrying its source and evidence status. Its own
verification reports are explicit that its passes carry zero
hardware-tested claims. This book therefore treats hardware-doc
device records as researched-only evidence about parts, never as
proof that Drew owns a given board. Ownership is established only
by the machine inventory above and the AliExpress aggregation —
both still to come.
