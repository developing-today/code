# Supplement — hardware source notes + memory extracts

Notes taken while reading the hardware sources and memory, 2026-11-23/24.
Where a fact is quoted, its source file is named. Personal identifiers in
the collected machine record (drive serial, MAC address) are deliberately
**not** reproduced here — the record itself lives in the code repo at
`doc/hardware/machines/amd.md` for anyone with repo access.

## A. `doc/platform/hardware.md` (code repo, draft built 2026-10-21)

Sourced only from the idea register, MEMORY.md, and two plan files; the
draft says so itself and flags its own gaps. Key points beyond the census:

- §1: WRX80E-SAGE named board; "Sources do not say whether every core box
  is this board or only one is."
- §4 networking table: switches row reads, verbatim in substance:
  "**Not specified in any source read.** The code repo has
  `doc/inventory/routing-and-switching.md` … its contents were not part of
  this pass; switch make/model/count is **MISSING** here until that file
  is folded in." — Census C folds it in (census §4).
- §5 ESP32 wing: "Per-device verification docs for ESP32 boards already
  live in developing-today/hardware-doc." Maker bench: ESP32-class
  hardware, Raspberry Pi, sensors, displays, logic analysers, buses,
  radios; LoRa, Reticulum, Thread, Matter, Zigbee, Meshtastic. "No
  per-device inventory of these exists in the sources read — MISSING."
- §7 prices: "No per-item prices, purchase prices, or relative price
  table exist in any source read for this draft."
- §6 personal hardware: Framework laptop (NixOS/KDE, Neovim + VS Code;
  4 TB Samsung 990 Pro per the Turso plan; designated canonical warm
  store + ingest factory for Atlas); work Mac (model/spec not in
  sources); OnePlus 12R; e-reader + Calibre on Omarchy (which machine:
  not in sources); Model F keyboards (hobby note, no inventory).

## B. Collected machine record — `doc/hardware/machines/amd.md`

- Collected: 2026-10-29T15:48:31Z by `machines/collect-inventory.py`;
  machine-readable twin `amd.json`; schema
  `machines/inventory-record.schema.json`.
- System: Framework Laptop 13 (AMD Ryzen 7040 Series) (A7), board
  Framework FRANMDCP07 (A7), BIOS INSYDE Corp. 03.18 (01/08/2026).
- CPU: AMD Ryzen 7 7840U w/ Radeon 780M — 8 cores / 16 threads.
- Memory: 132,752,572,416 bytes total (~128 GB); slot/ECC detail unread
  (dmidecode needs root).
- Drive: /dev/nvme0n1 Samsung SSD 990 PRO 4TB (serial in the source
  record; not reproduced here).
- Network: MediaTek MT7922 802.11ax wireless (MAC in the source record;
  not reproduced here). Unknowns recorded: link speed not exposed; no
  BMC discoverable; SMART/firmware unread without root.
- This is the **only** collected machine record in `machines/` (plus
  README, schema, template, collector script). No Threadripper record
  exists.

## C. `nixos/hosts/` (code repo)

- `nixos/hosts/default.nix` defines exactly two hosts: `nixos`
  (Framework 13-inch 12th-gen Intel, nvme0n1/2t, desktop profile) and
  `amd` (Framework 13-inch 7040-AMD, nvme0n1/4t, desktop + services incl.
  `services/lab-tunnel` — "Lab cluster Cloudflare Tunnel (code#6455):
  connector + edge status service for lab.security.cab",
  `services.lab-tunnel.enable = true`).
- Remaining files in `nixos/hosts/` are `amd-*` fragments (agent rebuild,
  IPv4 preference, krdp, openclaw secrets, rustdesk, sunshine).
- `nixos/hardware-configuration/` has framework/13-inch (12th-gen-intel,
  7040-amd) and pcengines/apu only.
- **No host definition exists for any basement/cluster machine.**

## D. hardware-doc structure (via the `doc/hardware` symlink)

- `devices/`: espressif, flipper-devices, lilygo, m5stack,
  nicolai-electronics, seeed-studio, soldered-electronics, waveshare,
  xteink, zerowriter.
- `devices/espressif/`: esp32-p4-eye, esp32-p4-function-ev-board,
  esp32-p4x-c5-function-ev-board, esp32-p4x-eye,
  esp32-p4x-function-ev-board, shared-artifacts.
- `devices/seeed-studio/`: xiao-esp32s3-sense.
- `devices/waveshare/`: e-paper-esp32-driver-board, esp32-c6-epaper-1.54,
  esp32-p4-wifi6-touch-lcd-3.4c, esp32-p4-wifi6-touch-lcd-4c,
  esp32-p4-wifi6-touch-lcd-xc-shared, esp32-s3-epaper-1.54,
  esp32-s3-epaper-3.97, esp32-s3-knob-touch-lcd-1.8,
  esp32-s3-photopainter, shared-artifacts.
- `devices/m5stack/`: cap-cc1101, cap-lora-1262, cap-lora868,
  cardputer-adv, cardputer-mesh-kit, cardputer-v1.0, cardputer-v1.1,
  cardputer-zero, dinmeter, dinmeter-v1.1, papermono, papermono-lite,
  stamp-c6lora, unit-c6l, unit-lora-e220, unit-lorawan-stm32wle5,
  shared-artifacts.
- `components/espressif/`: esp32-c3, esp32-c6-wroom-1, esp32-p4,
  esp32-s3fn8, esp32-s3r8, esp32-s3-wroom-1, esp32-s3-wroom-1u,
  esp32-u4wdh, esp32-wroom-32u, esp32-wrover.
- Root verification files: VERIFICATION-lilygo-t-display-k230.md,
  VERIFICATION-m5stack-cardputer.md, VERIFICATION-m5stack-dinmeter.md,
  VERIFICATION-m5stack-papermono.md, VERIFICATION-xiao-esp32s3-sense.md,
  VERIFICATION-xteink.md, VERIFICATION-xteink-round2.md,
  VERIFICATION-sweep-2026-09-04.md, verification.json,
  verification-summary.json.
- README research-pass table (paraphrase): Waveshare
  ESP32-S3-Knob-Touch-LCD-1.8 pass 2026-08-21→23 (~22 component records);
  ESP32-P4 + official boards pass 2026-08-21 (five board records; "no
  Korvo board exists on P4"); framework-guide extraction 2026-08-24
  (ESP-IDF ×5 target builds, ESP-ADF, ESP-IoT-Solution, esp-dev-kits —
  framework PDFs archived out, ~219 MB; datasheets/TRMs/errata/design
  files retained); NixOS embedded-host pass 2026-08-30 (ESP-IDF 5.5.2 +
  6.0.1; "Nothing in this pass was built or rebuilt"); seven-session
  sweep 2026-09-04→07 (~40 device + ~90 component records; Xteink,
  M5Stack, LilyGO incl. first RISC-V device K230; "Nothing in this pass
  was verified on physical hardware"); Xteink round two 2026-09-11
  (FCC grantee-code enumeration found the unannounced X4 Light; "Still
  nothing verified on physical hardware").
- Repo shape: hardware-doc is a standalone repo, symlinked into
  consumers, not a submodule; ~440 MB; sibling `repo-archive` holds bulk
  artifacts with `*.ARCHIVED.md` placeholders (SHA-256 + recovery URLs).

## E. Memory extracts

### `~/memory/topics/basement-cluster.md` (topic file, verbatim substance)

- "Old Talos cluster — TORN DOWN (Drew, 2026-10-26): the v1.9-era cluster
  behind doc/talos-addresses.md (VIP 10.10.0.42, a1/a2/b1 + a3/a4/b2–b6)
  and infra/talos/01b.us/ was only a test and has been torn down. Nothing
  from it is running; the current program starts from bare machines."
- Seeded 2026-10-26 (Drew approved): planning/roadmaps/basement-cluster.md
  (PR #1082), first status record 2026-10-27, 12 epics code#1080–#1093 in
  Project #4 (Area literally `lab-cluster`), 13 tasks code#1094–#1106,
  dashboard code#1108, `mode/*` labels on all 4 repos; nix#501–#508
  superseded-pointer comments both ways; closing them was Drew's call
  (they are now closed).
- Brain-dump summary (2026-10-20): ~12 Threadrippers, 15–16× 16 TB
  helium WD HDDs, ≥3 TB DDR4 ECC, 990 Pro best NVMe, GPUs negligible
  (3× Intel B50, 2060, old display cards); 3–5 machines Talos+Cozystack
  via TALM; Postgres CNPG, Cloudflare tunnel, Tailscale, NATS, RustFS,
  FluxCD; storage sketch RAID 10 / RAID 100 / DRBD+LINSTOR; hard rule no
  HashiCorp; basement power/mess block the full fleet; 2 Gbps Brightspeed;
  egress scarce; Turso $5 + Cloudflare $5 in play, $25–30 possible, $75
  out; OCI + B2 queued; full inventory/BIOS/BMC audit with routine KVM
  screenshot verification is a sooner task.

### `~/MEMORY.md` lines of record

- Home cluster line: "parent issue #6066; epics #1080–#1090; first lab
  Cloudflare Tunnel live 2026-11-06 (lab.security.cab). Drew's networking
  priority = router + Wi-Fi. Current LAN: Brightspeed fiber → Deco
  BE11000 router mode; IPv6 ULA-only (#1280). Oracle/OCI is NOT a backend
  (stood down 2026-10-31)."
- Backup topology (2026-10-27): "sandbox→laptop+R2+Drive;
  laptop→Drive+R2". "Sandbox is not storage (2026-10-27): backups belong
  on the machine itself or Google Drive, never the sandbox."

### Daily-log extracts

- `~/memory/2026-10-20.md`: Lab Ops Hub demo built + deployed
  (lab-ops-hub.developing-today.workers.dev); D1 labops-db "seeded with
  talos-1/2/3 machines matching the basement-cluster sketch"; R2 bucket
  labops-files; verified live end-to-end for the non-container paths;
  Cloudflare Containers flaky (account/platform-side start failures;
  in-Worker fallback crawler is the working path); PR #436 carried the
  demo into pkgs/lab-ops-hub.
- `~/memory/2026-10-27.md`: home IPv6 stalls at 0 bytes on payload
  transfers (curl -6 0 B/22 s vs curl -4 full 4 MB in 8 s); filed as
  code#1280; PR #1286 merged the laptop gai.conf IPv4-preference
  workaround; Drew directive: prefer IPv4 in every way until fixed.
- `~/memory/2026-11-04.md`: home lab story filed ~23:30 CST as the #6066
  ticket set (children #6070 racking verbatim, #6071 DXO10 ×3 verbatim,
  #6073/#6074/#6075 Dell switches, #6067 edge, #6068 mesh, #6077
  interconnect, #6076 generic containers, #6078 SBC investigation);
  TALM/Talos/Cozystack deduped to #1086/#1087 under #1817.
- `~/memory/2026-11-05.md`: Drew answered 2026-11-05 ~09:44 CST —
  (1) #6071 "DXO10" model confirmed = DX010 (log spells it "Celestia
  DX010"; inventory spelling is Celestica); the physical rack/power/label
  install is NOT today (stays blocked on Drew's physical step).
  (2) Drew's stated priority: "router and wifi networking first."

## F. Goal-workspace files read (lab-cluster goal)

`~/workspace/goals/stand-up-lab-cluster-talm-talos-cozystack-postgres-rustfs/`:
GOAL.md (goal_ba17a5777c21; execution tickets #1086+#1087 under #1817;
home-lab parent #6066); files/idea-register-2026-10-20.md (the register,
incl. addenda: ESP32 wing 07:04, firmware state + notes system 07:14,
spare PCs + quorum witnesses 2026-10-21, DNS register 2026-10-25,
brain-dump explosion amendment 2026-11-04, agent-notes incorporation
2026-11-04); files/project-structure-spec-2026-10-26.md (4-layer project
structure; mode labels; the 12-epic seed table);
files/cold-store-r2-layout-design-2026-10-25.md (Drive cold vault vs R2
hot tier; 5 named R2 buckets; Drive 5 TiB quota / 731.98 GiB used /
178.12 GiB Drive files at 2026-10-25 live check; 4–16 GB immutable
bundles; design-only disclaimer); files/oci-free-plan-2026-10-20.md
(2026 free-tier halving correction: A1 = 2 OCPU/12 GB; 200 GB block
storage binding; Chicago vs Ashburn region analysis);
hidden_files/research-brief-2026-10-03.md (talm/Talos/Cozystack/RustFS/
Postgres research with source URLs; talm repo-name correction to
cozystack/talm); hidden_files/epic-bodies/epic-01..12.md + maps +
epic-results.json (epic number → issue number mapping used throughout
the census); hidden_files/task-bodies/ (tasks 1.1–3.4 = #1094–#1106).
