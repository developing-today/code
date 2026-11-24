# Supplement — GitHub issue census (raw lists)

Gathered 2026-11-23/24 via the laptop fleet client only
(`~/tools/laptop-ssh.sh '~/.local/bin/fleet gh api "search/issues?q=..."'`
and direct `repos/developing-today/code/issues/<n>` reads).
No GitHub connector, no sandbox gh. States are as returned at gather time.

## Search: `talos repo:developing-today/code` (per_page=30, first page)

```
1086 [open] epic(cluster): TALM + Talos bootstrap
1817 [open] EPIC: Basement Talos/Cozystack cluster program
6066 [open] epic(lab): home lab build-out — racking, switches, edge routing, mesh Wi-Fi, cluster bring-up (tracking)
1087 [open] epic(cluster): Cozystack platform
1088 [open] epic(cluster): platform services
8873 [open] design(P9): Lab integration — DRAFT for Drew's ratification
1810 [open] Hardware inventory + hardware-stats gathering: pick the first 3–5 cluster machines
6077 [open] ops(lab): cluster interconnect — connect the dozen cluster machines + 2–3 mini PCs
1815 [open] Research: boats.dev
1083 [open] epic(cluster): basement power & space
9166 [open] fleet-ca: federation & infrastructure research — OpenBao, SecureStore, Cloudflare, Kubernetes (nonblocking)
1105 [open] task(cluster): install and verify the dedicated 20A circuits
902  [open] impl(dns-registry): recurring DNS backup job, renewal/cost tracking, dashboard view of the domain register
13905 [open] Code Book + Lab Book programme: whole-project book, per-book roadmap sections, lab book, Code Space
1091 [open] epic(cluster): cloud extension (OCI free fleet & egress offload)
1811 [open] BMC/BIOS configuration audit + routine KVM screenshot verification system
2179 [open] Machine pool: onboard additional machines as fleet job nodes and redesign placement around a pool
1813 [open] Storage pools: RAID10 HDD tier for RustFS + striped/mirrored NVMe tiers with LINSTOR/DRBD
5030 [open] design(cluster): egress strategy for 2 Gbps home fiber — budgets, tunnel topology, failure matrix
1080 [open] epic(cluster): hardware inventory & first-node selection
8000 [closed] fix(talos): save-secrets.sh encrypts atomically and fails loudly (#6554) [PR]
6554 [closed] talos(save-secrets.sh): a failed sops encryption truncates secrets.enc to zero bytes and the script still exits 0
5052 [closed] docs(infra/cluster): TALM/Talos per-node disk config answer + runbook (#1812) [PR]
1812 [closed] TALM bootstrap: declarative per-node disk config for non-identical drives
4904 [open] ops(cluster): OCI tenancy access + Always Free fleet provisioning (child of #1091)
1090 [open] epic(cluster): observability, backup & recovery
1082 [closed] docs(planning): seed basement cluster roadmap + first status record [PR]
1109 [closed] docs(basement-cluster): link filed epic issues in roadmap [PR]
```

## Search: `switch repo:developing-today/code` (lab-relevant hits only)

```
1097 [open] task(cluster): verify NIC and switch-gear facts
6075 [open] ops(lab): rack Dell switch 3 — rails, firmware, initial config, config stored, labels
6074 [open] ops(lab): rack Dell switch 2 — rails, firmware, initial config, config stored, labels
6073 [open] ops(lab): rack Dell switch 1 — rails, firmware, initial config, config stored, labels
6066 [open] epic(lab): home lab build-out (tracking)
6077 [open] ops(lab): cluster interconnect
6067 [open] ops(lab): reconfigure the edge routing and rewire
5765 [open] Epic: deployment architecture — services layer + deploy tool (tracking)
```

## Search: `rack repo:developing-today/code` (lab-relevant hits only)

```
6070 [open] ops(lab): another 4 each of racking (as dictated)
1106 [open] task(cluster): basement cleanup + rack layout for the first nodes
6071 [open] ops(lab): install 3 more Celestia DX010 units
1085 [open] epic(cluster): storage assembly & design validation
6078 was not in this page; see direct reads below
```

## Search: `esp32 repo:developing-today/code` (scoped; an unscoped
## `esp32` search returned only unrelated third-party repos and was discarded)

```
1093 [open] epic(cluster): ESP32 wing
13647 [open] id: embedded presenter slice — rotary-knob input: the knob as a carrier for the touch verb vocabulary (S3-Knob-Touch-LCD-1.8 / P4-Knob-Touch-AMOLED-1.75)
5783 [open] id: design: ESP32 hardware platform line — boards, provisioning, firmware lifecycle, OTA, power
13646 [open] id: embedded presenter slice — e-paper presenter (M5Paper class): refresh discipline and ambient-first cards on monochrome e-paper
13632 [open] id: embedded presenter slice 1 — round-panel hello-world presenter (GC9A01 + CST816S, bridge-fed glance card, touch back-channel)
13643 [open] id: embedded presenter slice (Tanmatsu class A) — world presenter as an AppFS app on ESP32-P4
13910 [open] Lab: aggregate AliExpress purchase history into a hardware inventory
10417 [open] replicate: format auto-fix workflow to dezren39/muse-esp32-screen (from code PoC #2015; target repo has issues disabled)
6279 [open] id(phase-14): More shells (mobile, native, hardware) — core topic (research + design)
6298 [open] id(phase-14): More shells (mobile, native, hardware) — v3 — outline: hardware shell
12942 [closed] id-hw-shell: ESP32-class hardware shell spike (#10929) [PR]
13635 [closed] research(#13630): embedded display tier — ESP32 screens as world presenters [PR]
8001 [closed] docs(id): decision record for ESP32 hardware platform line (#5783) [PR]
13630 [closed] id: research/design: embedded display tier — ESP32 screens (tiny, round, touch) as world presenters
10929 [closed] id(phase-14): shells v1 slice 3 — ESP32-class hardware shell spike (#6296)
10931 [open] id(phase-14): shells v1 slice 5 — conformance-suite packaging for v1 shells (#6296)
2056 [open] epic: ID platform expansion — apps, comms, currency, hosting, boards (2026-10-28 directive, LIST 6–15)
```

## Search: `lab repo:developing-today/code label:epic/lab-cluster` (per_page=40)

```
6066, 6068 (mesh Wi-Fi rewire), 6071, 6067, 8873, 6070, 6077, 6075, 6074, 6073,
1105, 1811, 1817, 1096, 6455 [closed] first lab Cloudflare Tunnel,
1086, 5772 (VMs: declarative guest definitions), 1081, 1108 [closed] dashboard,
1810, 1088, 8408 (egress enforcement), 5030, 1102, 4904, 1085, 1095, 1091,
1097, 1100, 1093, 1087, 1092
```

## Search: `cluster repo:dezren39/fleet`

```
60 [closed] fork-migration-plan: archive org resolved; pure-clone archive move executed [PR]
```
(Only hit — no lab/cluster programme material lives in dezren39/fleet issues.
No developing-today-forks/* issue hits for the lab terms were returned by the
searches run; the lab programme's issue home is developing-today/code.)

## Direct reads (title + state) — epic tasks #1094–#1106 and others

```
1094 [closed] task(cluster): design the hardware inventory record (schema + template)
1095 [open] task(cluster): gather stats - core Threadripper machines
1096 [open] task(cluster): gather stats - remaining machines + boxed HDD baseline
1097 [open] task(cluster): verify NIC and switch-gear facts
1098 [open] task(cluster): rank machines and name the first 3-5 nodes
1099 [open] task(cluster): collect current BIOS/BMC versions and settings - core machines
1100 [open] task(cluster): define the golden firmware/settings baseline
1101 [open] task(cluster): bring core machines to the golden baseline
1102 [open] task(cluster): routine KVM screenshot verification + reappliable config
1103 [closed] task(cluster): circuit spec + electrician brief
1104 [open] task(cluster): engage an electrician (quotes + schedule)
1105 [open] task(cluster): install and verify the dedicated 20A circuits
1106 [open] task(cluster): basement cleanup + rack layout for the first nodes
1108 [closed] impl(dashboard): project page + complexity-weighted progress visualization (seed: lab-cluster)
1810 [open] Hardware inventory + hardware-stats gathering: pick the first 3–5 cluster machines
1811 [open] BMC/BIOS configuration audit + routine KVM screenshot verification system
1813 [open] Storage pools: RAID10 HDD tier for RustFS + striped/mirrored NVMe tiers with LINSTOR/DRBD
4904 [open] ops(cluster): OCI tenancy access + Always Free fleet provisioning (child of #1091)
6455 [closed] impl(cluster): first lab Cloudflare Tunnel — connector on amd, lab.security.cab seed ingress
6070 [open] ops(lab): another 4 each of racking (as dictated)
6072 [closed] research(gpu-capacity): GPU options after Modal credits — Blacksmith (none), GitHub Actions GPU runners, Copilot (none), OCI (stood down), local (none owned)
6078 [closed] research(lab): are Raspberry Pis / SBCs worth anything in the lab? measured perf-per-watt vs cluster nodes + mini PCs
```

## Direct reads — bodies captured in full for the census

- #6066 (build-out sequence, Dell ×3, DX010 ×3, racking, 4th-switch note,
  verbatim-item flags), #6077 (interconnect acceptance), #6076 [closed,
  status:done] (generic containers), #1812 [closed] (TALM disk question),
  #8873 (P9 draft ratification), #6073 (per-switch step list; model NOT
  stated — to be recorded on the ticket), #6070 (verbatim dictation +
  interpretation), #6067 (edge routing), #13910 (AliExpress intent + shape),
  #13920 (switches inventory completion — first slice definition),
  #6078 [closed] (SBC research question + acceptance; checkboxes unchecked
  in the body), #8408 (egress enforcement scope), #5772 (VM guests; recorded
  current state: libvirtd enabled, zero guest definitions).

## Predecessors — dezren39/nix#501–#508 (all closed, superseded pointers)

```
501 [closed] Hardware inventory + hardware-stats gathering: pick the first 3–5 cluster machines
502 [closed] BMC/BIOS configuration audit + routine KVM screenshot verification system
503 [closed] TALM bootstrap: declarative per-node disk config for non-identical drives
504 [closed] Storage pools: RAID10 HDD tier for RustFS + striped/mirrored NVMe tiers with LINSTOR/DRBD
505 [closed] Egress strategy: 2 Gbps home fiber behind Cloudflare/Turso/Tailscale
506 [closed] Research: boats.dev
507 [closed] OCI always-free maximization: account setup + automation
508 [closed] EPIC: Basement Talos/Cozystack cluster program
```
Relocated/successor mapping of record: nix#503 → code#1812 (relocated
2026-10-28, "filed in the wrong repo"); nix#501 → code#1810; nix#502 →
code#1811; nix#504 → code#1813; nix#507 → code#1091/#4904 line.
