# Hardware — Drew's Fleet, Workstation, and Agent Computer

Draft built 2026-10-21. **Sourced only** from:

- `~/workspace/goals/stand-up-lab-cluster-…/files/idea-register-2026-10-20.md`
  (the basement brain-dump register — the primary hardware source)
- `~/MEMORY.md` (basement-cluster summary, hardware-state note, About
  Drew, VM disk layout)
- `~/workspace/your_files/supabase-muse-dev-plan.md` and the Turso
  Pro-month plan (laptop role)

Where a file gives no number, this document says so. **Relative
prices are almost entirely MISSING from the sources** — see §7.
Nothing here is a secret: Drew has stated this inventory is not
sensitive.

---

## 1. Basement fleet — the Threadripper machines

| Item | What the sources say |
|---|---|
| Count | **~12 machines** ("a dozen") |
| Platform | Threadripper. Core machines are **identical motherboard, same CPU, same specs — except SSDs** |
| Named board | **ASUS Pro WS WRX80E-SAGE** ("WRX80 Sage") — firmware update applied (2026-10-20 note). Sources do not say whether every core box is this board or only one is |
| Dell machines | Some Dell machines were also picked up; **no firmware updates could be found for them**; BIOS got a best effort |
| State today | **No OS installed** (deliberate — nothing hand-built that "just existed forever"). Every machine's CPU at least boots; **all RAM works**; **nothing load-tested yet** |
| Remote management | Core machines have **enterprise BMC/IPMI** (dictated as "ASM" / built-in enterprise KVM) — versions and every setting still to be audited; first cluster task is confirming core machines are setting-for-setting identical |
| Blockers | **Basement power capacity and general mess.** Getting all 12 running is a long tail; the build target is **3–5 machines first** (best / most-identical first) |
| Planned OS/stack | Talos Linux + Cozystack, bootstrapped declaratively with TALM; Postgres (CNPG), RustFS, NATS, FluxCD, Cloudflare Tunnel, Tailscale, DRBD+LINSTOR. **Hard rule: no HashiCorp anything** |

Existing hardware documentation (not re-copied here):

- developing-today/code repo, `doc/` tree on main: `doc/hardware`
  (incl. `routing-and-switching.md`, enrichment checklist, gap
  analysis, standard attributes) + a `.lootbox/scripts/*hardware*`
  doc pipeline and a `hardware-device-research` agent skill.
- **developing-today/hardware-doc** (~550 MB): per-device
  `VERIFICATION-*.md` files — mostly ESP32 boards today; the natural
  home for the BMC/BIOS audit output.

## 2. Storage

| Item | What the sources say |
|---|---|
| HDDs | **15–16 × 16 TB** helium Western Digital server pulls (Drew also said "at least 12" once — register records both). Bought **before storage prices spiked** (the only price-timing note in any source) |
| HDD state | **8 are still in the box (uninstalled)** — no SMART/baseline data exists for the HDDs yet |
| RAM | **≥ 3 TB DDR4 server ECC RAM** across the fleet. All works; not load-tested |
| NVMe / SSDs | Every machine has some consumer NVMe boot drive; best are prosumer **Samsung 990 Pro**. Mid tier: 980s, smaller 990s, other good consumer SSDs. Per-machine SSDs are the one spec that differs between core boxes |
| Firmware/SMART data | A **backup hard drive holds SMART data + manufacturer firmware data for every NVMe** (not the HDDs). Firmware updates were run wherever findable |

### Storage design sketch (Drew's, to be validated)

- **HDD tier:** 3 hosts × 4 × 16 TB in **RAID 10** → RustFS cold-ish
  buckets.
- **Fast NVMe tier:** all Samsung 990s.
- **Mid NVMe tier:** striped ×3 (≈990-class performance) × mirrored
  ×2 — **"RAID 100"** (3 stripes, 2 mirrors, 6 drives) — two drives
  must die to down a machine.
- With 3 machines replicating via DRBD/LINSTOR, data loss needs the
  same failure on 2 machines at once.
- Two named pools from earlier work: (1) 4 drives RAID 10;
  (2) 6 drives RAID 100.
- Goal: 3–5 devices with near parity in disk size and capability.
- Open design question (Drew's): how to declaratively configure, in
  Talos/TALM, hard drives that are all different per machine (TALM's
  per-node hardware harvest is the presumed answer — flow still to
  be proven).

## 3. GPUs

Explicitly **not a compute resource** — no local model-farm plans:

- **3× Intel B50**
- **1× RTX 2060** (noted: low hash rate)
- Plus very old display cards, useful only for basic GUI/BIOS access
  on a misbehaving box

## 4. Networking

| Item | What the sources say |
|---|---|
| Core-machine NICs | High-speed NICs; dictated as **"at least 200 Gb ports"** — **exact spec TBD in inventory**; register's reading: likely dual high-speed ports per core box. **This is an open question, not a confirmed spec** |
| Switches | **Not specified in any source read.** The code repo has `doc/inventory/routing-and-switching.md` (named in the register) — its contents were not part of this pass; switch make/model/count is **MISSING** here until that file is folded in |
| Home WAN | **2 Gbps symmetric fiber, Brightspeed** — the only fiber provider in his part of Osseo. A second line would make 4 Gbps. Cable is 100 Mbps and more expensive (possible redundancy pickup). TriCounty Communications fiber may reach his part of Osseo later — then buy both fibers |
| Backup/emergency | Hotspot plans and Starlink investigated; Starlink would likely need business plans to avoid cancellation for this use |
| Egress doctrine | Egress is the scarce resource, not compute: tunnel/shunt bulk traffic to Cloudflare / Turso / Tailscale and other cheap/free-egress SaaS; compress aggressively. The fleet's value = fast CPUs, huge RAM/storage at ~zero marginal cost vs renting equivalent cloud boxes |
| House | Plans to stay in the Osseo house long-term (10+ years, possibly 40) — home Ethernet hardwiring is a standing project |

## 5. ESP32 wing

- A whole separate **low-end section alongside the high-end compute**:
  ESP32-class boards to be connected to the ID project, running
  **sensors and LoRa**.
- Deliberately a side project of a side project for now — noted, not
  scheduled.
- Per-device verification docs for ESP32 boards already live in
  developing-today/hardware-doc (§1).
- Drew's broader maker bench (from MEMORY's About Drew): ESP32-class
  hardware, Raspberry Pi, sensors, displays, logic analyzers, buses,
  radios; networking tech incl. LoRa, Reticulum, Thread, Matter,
  Zigbee, Meshtastic. No per-device inventory of these exists in the
  sources read — **MISSING** beyond the hardware-doc repo pointer.

## 6. Personal / workstation hardware

| Item | What the sources say |
|---|---|
| Framework laptop | Drew's primary personal workstation: **Framework laptop running NixOS, KDE Plasma**; editors Neovim + VS Code. In the Turso Pro-month plan it is specified further: **4 TB Samsung 990 Pro**, always-on, assumed non-failing for planning — designated **canonical warm store + ingest factory** for Atlas (full corpus, SQLite shards, embedding compute, consolidation). Tailscale access for Muse via the laptop: promised by Drew, pending (2026-10-20) |
| Work Mac | From his Vertex job (confirmed by Drew 2026-10-19): a Mac running Python 3.13/Poetry work tooling (the "viper" CLI etc.). Model/spec: **not in sources** |
| Phone | **OnePlus 12R**, dual nano-SIM (US Mobile) plus eSIM. Budget mobile setup: US Mobile by-the-gig/light plans, multi-network |
| Reading | E-reader + Calibre library (Calibre set up on Omarchy Sep 2026). Omarchy (Arch-based Linux) runs on at least one machine alongside NixOS — which machine: **not in sources** |
| Keyboards | Mechanical keyboards, especially vintage Model F and customization (hobby note; no inventory) |

## 7. Prices — what is and isn't in the sources

**No per-item prices, purchase prices, or relative price table exist
in any source read for this draft.** Specifically:

- HDDs: only "bought before storage prices spiked" (relative timing,
  no figures).
- Threadripper machines / motherboards / RAM / NVMe / GPUs / NICs:
  **no prices at all** in the register or MEMORY.
- Framework laptop: capacity of its SSD (4 TB 990 Pro) is stated; its
  price is not.
- The only dollar figures anywhere near the hardware program are
  *cloud* budget tiers (Turso $5 / Cloudflare $5 in play, $25–30
  later, $75 out) — those belong to the plan docs, not this one.

If a priced inventory is wanted, the likely home of the raw data is
the code repo's `doc/hardware` tree / hardware-doc repo (§1), which
were named but not read in this pass — flagged in the summary as a
follow-up source, not guessed at here.

## 8. Muse's own computer (the sandbox VM)

For completeness — the machine this agent runs on (MEMORY, 2026-09-29):

- **100 GB disk (encrypted btrfs) at /home/hatch** — the persistent
  home for workspace, tools, and memory; the only thing that survives
  a sandbox recycle.
- 7.6 GB ephemeral OS layer (/), 7.5 GB container-runtime backing
  store, 801 MB unmounted boot/config partitions; `/opt/hatch` is a
  read-only squashfs of platform runtime tools.
- Builds here are CPU-throttled (observed ~0.86 MB/s xz on 2 throttled
  vCPUs during the nix-cache benchmark) — a sandbox limit, not a plan
  limit; a bigger Muse plan does not change this computer.

## Sources for this draft

- `~/workspace/goals/stand-up-lab-cluster-…/files/idea-register-2026-10-20.md`
  — §§Facts (hardware, network), Storage design, Inventory system,
  Addendum 2 (firmware state).
- `~/MEMORY.md` — basement cluster summary line; hardware-state note
  (backup drive, 8 HDDs boxed, WRX80 Sage, Dell firmware gap); About
  Drew (Framework/NixOS workstation, work Mac, OnePlus 12R, Omarchy);
  VM disk layout (2026-09-29).
- `~/workspace/turso/pro-month-plan.md` — Framework laptop spec
  (4 TB 990 Pro) and its Atlas role.
- `~/workspace/your_files/supabase-muse-dev-plan.md` — hardware
  inventory named as a Supabase use case (no additional hardware
  facts).
