# ESP32

The microcontroller end of the lab: ESP32-family boards as an ID
hardware shell — sensors and LoRa connected to the ID project —
under epic
[#1093](https://github.com/developing-today/code/issues/1093)
("ESP32 wing", open, horizon: Ideas). Drew's own framing, repeated
in every source: **"a side project of a side project"** — noted
and lightly planned, not resourced ahead of the core build. The
first genuinely useful slice on record is basement sensor
coverage (power, temperature) feeding the cluster's observability
epic [#1090](https://github.com/developing-today/code/issues/1090).

This chapter's status, stated at the top because everything else
hangs on it: **researched-only and designed, with two software
proofs, and zero physical-hardware verification anywhere in the
record. No board is yet recorded as bought, flashed, or run.**
No record states that any specific ESP32 board has been flashed,
run, or even confirmed owned. See gap 11 in
[Gaps and open questions](../plans/gaps-and-open-questions.md).

## The verified part names

Drew dictated "ESBs 3 and ESBP 4" for this chapter. Both readings
are **verified from the record, not guessed**:

### ESP32-S3 — the reference part

- Named as the reference chip/board class in a landed design,
  `doc/2026-11-09_design_id5783.md`: "a single reference board —
  ESP32-S3 devkit-class (16 MB flash / octal-PSRAM class, native
  USB)". The decision record landed via PR #8001 for
  [#5783](https://github.com/developing-today/code/issues/5783).
- Named as the tier's reference chip in landed research,
  `doc/2026-11-23T18-55-21Z_research_id13630_embedded_display_tier/`:
  "The tier's reference chip — every first-wave target below is
  S3". First hardware target: the **Waveshare
  ESP32-S3-Touch-LCD-1.28** (ESP32-S3R2). Research issue
  [#13630](https://github.com/developing-today/code/issues/13630)
  closed via PR #13635.
- Chip facts of record: Xtensa LX7 dual-core at 240 MHz, 512 KB
  SRAM, WiFi 4 + BLE 5.0, native USB.
- Silicon records exist in hardware-doc: `esp32-s3r8`,
  `esp32-s3fn8`, `esp32-s3-wroom-1`, `esp32-s3-wroom-1u` — research
  records about parts, not ownership records (see the hardware-doc
  caveat in the [Hardware register](../site/hardware-register.md)).

### ESP32-P4 — the named future tier

- Named in the display-tier research as the future larger-panel
  tier: MIPI-DSI and a hardware JPEG codec suit panels above
  320×240 and RGB panels, at the price of a two-chip design — the
  P4 has no on-chip radio; Wi-Fi comes via an ESP32-C6 companion
  over ESP-Hosted.
- Chip facts of record: RISC-V dual-core at 400 MHz, 768 KB SRAM,
  no on-chip radio.
- Five official Espressif P4 board records exist in hardware-doc
  (`esp32-p4-eye`, `esp32-p4-function-ev-board`,
  `esp32-p4x-c5-function-ev-board`, `esp32-p4x-eye`,
  `esp32-p4x-function-ev-board`), and the research pass of record
  established the real P4 board lineup — no Korvo board exists on
  P4.
- The P4 is also targeted in one slice today: as the application
  processor of the Tanmatsu, below.

## The Tanmatsu — "Tenmazu", resolved

The dictated "tenmazu" is the **Tanmatsu**, resolved from the
record: a whole-repository search for "tenmazu" returns nothing;
the device is spelled Tanmatsu. It is a physical device: a
handheld terminal from the Nicolai Electronics / badge.team
lineage (it began as the WHY2025 badge design and is sold
commercially), which Drew named as **the reference design for
making worlds work "as an app" on this tier**. Its review is §9
of the #13630 research record.

| Part                   | What it is                                                                                            |
| ---------------------- | ----------------------------------------------------------------------------------------------------- |
| Application processor  | ESP32-P4 (dual RISC-V at 400 MHz, 768 KB SRAM, 32 MB PSRAM, 16 MB flash) — runs the launcher and apps |
| Radio coprocessor      | ESP32-C6 module (WiFi 6, BLE 5.3, IEEE 802.15.4) over ESP-Hosted (SDIO)                               |
| Management coprocessor | WCH CH32V203 (keyboard matrix and power management, I2C peripheral to the P4)                         |
| Display                | 3.97-inch 800×480 MIPI-DSI (ST7701S)                                                                  |
| Input                  | 69-key QWERTY keyboard (no touchscreen)                                                               |
| Extras                 | LoRa module (Ai-Thinker Ra-01S/SH, 433 or 868/915 MHz) on the C6 side; microSD slot                   |
| Licensing              | Software MIT; hardware CERN-OHL-P-2.0                                                                 |

The app system the tier adopts from it: the tanmatsu-launcher
plus **AppFS** (badge.team) — an app is a standard, unmodified
ESP-IDF binary with slug/title/version metadata in an 8 MB AppFS
partition; apps arrive via the on-device Hatchery store, BadgeLink
over USB (also the provisioning route), host-side image tools
(apps embedded in the firmware image at build time), or as the
main firmware itself (kiosk boot). Adopted shapes:
presenter-as-an-app, slug-keyed data, BSP-shaped hardware access,
a host management protocol.

One honest gap recorded in the research: stock AppFS bootloader
support covers ESP32, ESP32-C6 and ESP32-P4 — **not yet the
ESP32-S3**, this tier's first target. Adopting AppFS literally on
the S3 presenter waits on an upstream bootloader port; on
P4-class hardware it works today. A hardware-doc device family
for Nicolai Electronics exists
(`doc/hardware/devices/nicolai-electronics/`).

## The ESP32 platform design (#5783) — designed, decision-ready

`doc/2026-11-09_design_id5783.md` — "Design: ESP32 hardware
platform line — boards, provisioning, firmware lifecycle, OTA,
power". Status in the record: **decision-ready proposal — design
only; no firmware is implemented here.** Issue #5783 is open,
under ID epic
[#2056](https://github.com/developing-today/code/issues/2056).
Its decisions:

- **Board:** one reference board (ESP32-S3 devkit-class) plus a
  written compatibility envelope (Wi-Fi, at least 16 MB flash,
  PSRAM, hardware RNG, USB flashing without an external UART
  bridge) — no supported family in v1. Rejected as reference:
  ESP32 classic (no native USB, thinnest RAM margin; eligible
  later in a WROVER/PSRAM variant), ESP32-C3/C6 (RAM too tight
  for the TLS plus frame budget; named revisit), ESP32-S2
  (single core, no BLE).
- **Identity (the ID tie-in):** a device-owned keypair generated
  on-device at first boot; enrolment binds it through the
  [#5884](https://github.com/developing-today/code/issues/5884)
  setup ceremony; factory reset **destroys** the key and
  re-enrols as a _new_ device; a stolen device yields only its
  own key, expiry-bounded cached grants and its journal — never
  user keys.
- **Lifecycle:** Nix-flake reproducible builds in CI,
  offline-signed images, A/B partitions with automatic rollback
  to last-good, OTA fetched over HTTPS (never inside MQTT
  frames), a checksummed journal ring across reconnects.
- **Firmware language:** Rust on esp-idf, sharing the workspace's
  wire-bounds constants and manifest schema types with the bridge
  so the 64 KiB / 8 KiB limits are one definition. C/Arduino and
  bare-metal esp-hal + embassy-net both rejected for v1
  (embassy-net with a named revisit).
- **Power:** the v1 reference build is mains-powered ("Drew's
  bench"); battery is a budgeted envelope, **not a tested
  claim** — invariant I-10 forbids shipping a battery-life number
  without a measured profile on the reference board. Wake-storm
  avoidance is a fleet property (jittered, per-device-seeded
  backoff on the bridge's 1 Hz `time.tick` floor).
- **Field posture:** fail-closed; distrust power, network, broker
  and time in that order; the device is never a time authority
  (the bridge assigns ingest time); bounded store-and-forward
  with explicit gap signalling; never clicks through captive
  portals.

## The embedded display tier research (#13630) — researched, slices filed

`doc/2026-11-23T18-55-21Z_research_id13630_embedded_display_tier/`
— "Embedded display tier: ESP32 screens as world presenters"
(issue #13630 closed; PR #13635). Chip table of record: ESP32-S3
(reference); original ESP32 (supported envelope); ESP32-C3 (cost
floor, bridge-only class); ESP32-P4 (named, not targeted — except
via the Tanmatsu, above). The research also records the iroh
project's ESP32 walkthrough — ESP32 microcontrollers as QUIC
endpoints — as a transport data point for the tier. Filed
follow-on slices, all **open**:

- [#13632](https://github.com/developing-today/code/issues/13632)
  — presenter slice 1: round-panel hello-world presenter (GC9A01
  - CST816S, bridge-fed glance card, touch back-channel) — the
    Waveshare S3-Touch-LCD-1.28 target.
- [#13643](https://github.com/developing-today/code/issues/13643)
  — presenter slice (Tanmatsu class A): world presenter as an
  AppFS app on ESP32-P4.
- [#13646](https://github.com/developing-today/code/issues/13646)
  — e-paper presenter (M5Paper class): refresh discipline,
  ambient-first cards.
- [#13647](https://github.com/developing-today/code/issues/13647)
  — rotary-knob input slice (S3-Knob-Touch-LCD-1.8 /
  P4-Knob-Touch-AMOLED-1.75 named in the title).

## Projects and proofs — software only

- **MQTT device bridge — spec plus labelled proof of concept,
  proven in code (no hardware):** the design record for
  [#2343](https://github.com/developing-today/code/issues/2343)
  defines the `id.device.v1` topic schema (retained manifest
  only; `wants` never retained; TLS plus per-device credentials,
  fail-closed, no-oracle refusals; 64 KiB documents / 8 KiB args;
  a 1 Hz minimum `time.tick` chosen to avoid wake storms on
  ESP32-class radios). The proof of concept is
  `pkgs/id/src/device_bridge.rs` with `FakeBroker`/`FakeDevice`
  and integration tests. Home Assistant interop is
  out-of-scope-but-supported: HA may bridge to the same topics;
  the ID capability model stays authoritative. Production
  hardware rollout stays deferred
  ([#2028](https://github.com/developing-today/code/issues/2028));
  boards, firmware, OTA, ESPHome and power are #5783's.
- **ESP32-class hardware shell spike — done as a spike:**
  [#10929](https://github.com/developing-today/code/issues/10929)
  closed via PR #12942. The shell is specified as a capability
  statement (presence, journal-head, and the interaction subset).
- **A separate ESP32 screen repository exists:**
  `dezren39/muse-esp32-screen` —
  [#10417](https://github.com/developing-today/code/issues/10417)
  (open) replicates a format auto-fix workflow to it. The census
  cites its existence from the issue title; its contents were not
  read, and this book claims nothing about them.
- **Capability-taxonomy groundwork:** a design investigation
  defines one wire contract across web, ESP32/MQTT and text/audio
  (an ESP32 with GNSS serves `loc.position` at about 5 m;
  `speech.capture` exists for devices that cannot run speech to
  text — "most ESP32s"). A provider-security record flags the
  open hardware-attestation question for the ESP32 class: a
  provider key in extractable flash is cloneable, so until the
  secure-element decision is taken, ESP32-class instances are
  ceilinged as if cloneable.

## Maker-bench context

Drew's broader bench includes ESP32-class hardware, Raspberry Pi,
sensors, displays, logic analysers, buses and radios; networking
technology including LoRa, Reticulum, Thread, Matter, Zigbee and
Meshtastic (memory of record). No per-device inventory of any of
it exists — a gap (gap 11), and the AliExpress aggregation
([#13910](https://github.com/developing-today/code/issues/13910))
is the intended remedy.

The hardware-doc microcontroller corpus is wide — Seeed XIAO
ESP32S3 Sense, Waveshare S3/P4/C6 boards, the M5Stack Cardputer
family and its LoRa expansions, LilyGO T-Display-S3, Xteink
e-readers, and more — and every verification report in it carries
the same caveat this chapter opened with: **0 hardware-tested
claims**. It is research about parts. It is not a shelf inventory.
