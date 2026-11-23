# Embedded display tier: ESP32 screens as world presenters

Status: research/design record for #13630 (direction set by Drewry, 2026-11-23: support
embedded screens of varied sizes — very small, round, touch-enabled — as a world-presenter
tier of their own, separate from terminal/text rendering). The recommendations in §3, §4,
and §7 are **proposed, not decided** — they are inputs to the presenter program, whose
record ratifies or amends them. The survey facts (§2, §3, §6) stand on their own and also
feed the licensing program (license checker #5985): every component below carries its
license, and §6 states the bundling rule the survey implies.

Scope guard, from the issue: this tier is a **sibling presenter**, not a variant of the
terminal presenter. The terminal track (#13623, kitty stills pipeline) is untouched and not
blocked by anything here. The conversion findings of #5971 are inputs, reused where they
apply — most importantly its placement rule, which this tier inverts in an instructive way
(§3.3).

## 1. Where this starts: the presenter model already exists

Done check against main at the time of writing (2026-11-23):

- **The boundary is already drawn.** The #786 design (`doc/2026-11-01_design_id786.md`)
  fixes the presenter contract this tier inherits: the presenter is the adapter that owns
  the output device; it probes capability and keeps the resulting profile *local*; tier
  selection affects rendering only — never journal contents, world authority, or the wire
  contract; and a guest may never supply device-level output bytes (there, kitty/sixel
  escapes; here, panel rasters or LVGL objects). The SSH and iroh TTY presenters are the
  existing siblings. An embedded presenter is a third sibling that happens to run on a
  microcontroller: it consumes the world's view projection, renders it on its panel, and
  returns typed input events.
- **Input negotiation has a model.** #787 (kitty keyboard, slices A–F, landed) negotiates
  input capability per connection with a bounded probe and typed events, holding bytes
  until the capability is proven. The touch back-channel in §4 follows the same discipline:
  a bounded pairing ceremony, then a small closed vocabulary of typed events — nothing
  free-form reaches the world.
- **The "IDF tools" thread is resolved.** #5971 §1 recorded that no "IDF" converter family
  could be identified; Drewry confirmed the referent is ESP-IDF (Espressif's IoT
  Development Framework), and that the embedded surface is its own track (#13630). This
  record is that track.
- **Nothing embedded exists in the repo yet.** No ESP-IDF project, firmware tree, or
  embedded presenter record exists on main; the tier starts from this document.
- **One thing this record deliberately does not design:** the world-side *view contract*
  for glanceable cards. The terminal presenters consume a cell-grid surface
  (`WorldSurface`); a 240×240 round panel needs a state-card projection instead. §5 fixes
  what the device renders and returns; the projection's wire shape is named as an open
  question (§8) for the slice line, not invented here.

## 2. Hardware envelope

### 2.1 Chips

| Chip | CPU | SRAM | PSRAM (typical modules) | Radios | USB | Place in this tier |
| --- | --- | --- | --- | --- | --- | --- |
| ESP32-S3 | Xtensa LX7 dual-core @ 240 MHz, vector (SIMD) instructions | 512 KB | 2–16 MB (quad or octal; N16R8 module = 16 MB flash + 8 MB octal) | WiFi 4 (2.4 GHz), BLE 5.0 | Native USB OTG + USB Serial/JTAG | **The tier's reference chip** — every first-wave target below is S3 |
| ESP32 (original) | Xtensa LX6 dual-core @ 240 MHz | 520 KB | 0–4 MB (WROVER modules carry PSRAM) | WiFi 4, BLE 4.2 + BT Classic | None on-chip (boards use a USB-UART bridge) | Supported envelope: huge installed base of display boards; weaker (no SIMD for JPEG, no native USB, BLE 4.2) |
| ESP32-C3 | RISC-V single-core @ 160 MHz | 400 KB | Usually none in-package | WiFi 4, BLE 5.0 | USB Serial/JTAG | Cost floor for very small panels; single core and no PSRAM put it in the bridge-only, LAN class (§4) |
| ESP32-P4 | RISC-V dual-core @ 400 MHz | 768 KB | Up to 32 MB | **None on-chip** (WiFi via an ESP32-C6 companion over ESP-Hosted) | USB 2.0 HS OTG | Named, not targeted: MIPI-DSI and a hardware JPEG codec make it the future larger-panel tier (>320×240, RGB panels), at the price of a two-chip design |

Sources: ESP32-S3 datasheet facts as compiled in the Zephyr board documentation and the
ESP32-S3 variant survey (512 KB SRAM, BLE 5.0, USB OTG + Serial/JTAG, module PSRAM/flash
options); iroh's compatibility matrix for the P4's companion-radio arrangement.

### 2.2 Panels and touch controllers to support first

All first-wave panels are **controller-based** (ST7789/GC9A01/SSD1306 class): the panel
holds its frame in its own GRAM, the MCU streams regions into it, and ESP-IDF's `esp_lcd`
component is the uniform driver surface. ESP-IDF ships ST7789 and SSD1306 drivers in-tree;
the rest live in the Espressif Component Registry.

| Panel class | Example geometry | Driver path | Touch pairing | Notes |
| --- | --- | --- | --- | --- |
| Round | GC9A01, 1.28″, **240×240** | `espressif/esp_lcd_gc9a01` (registry, Apache-2.0, all targets) | CST816S | The constraint-first viewport: §5 designs against it |
| Small rectangular | ST7789 family: 135×240 (1.14″), 240×240 (1.3″/1.54″), 240×280 (1.69″), 240×320 (2.0″) | `esp_lcd` in-tree ST7789 | CST816S or FT6x36 where fitted; many modules are display-only | One driver covers the whole rectangular small end |
| Mid rectangular | ILI9341-class, 320×240 (2.8″) | ILI9341 driver in current ESP-IDF | FT6336 (FT6x36 family) | The roomy end of "small"; where the Freenove/LCDWiki S3 boards sit |
| Very small | SSD1306 OLED, **128×64** mono | `esp_lcd` in-tree SSD1306 (I2C or SPI) | None (typical) | The floor: 1 KB framebuffer, text/ticker class, display-only presenter (§5.4) |

Touch controllers, both I2C, both with official Espressif `esp_lcd_touch` drivers:

| Controller | Points | Interface / address | Gesture support | Driver |
| --- | --- | --- | --- | --- |
| CST816S (Hynitron) | 1 | I2C, 0x15 | **In hardware**: slide up/down/left/right, single click, double click, long press (register 0x01 gesture ID) | `espressif/esp_lcd_touch_cst816s` (ESP-BSP) |
| FT6236 / FT6336 (FocalTech FT6x36 family) | Up to 2 | I2C, 0x38 | Points + press/release events; gestures composed host-side | `espressif/esp_lcd_touch_ft5x06` (covers the FT5x06/FT6x36 family, FT6236 named) |

GT911-class controllers (larger capacitive panels) exist in the same driver family and are
named for the P4/larger-panel future, not the first wave.

### 2.3 Framebuffer, SPI, and refresh realities

- **Nobody holds a full framebuffer on the MCU.** The panel's GRAM is the framebuffer.
  LVGL renders into small draw buffers and flushes only dirty rectangles
  (`esp_lcd_panel_draw_bitmap` regions). The reference sizes, RGB565: a full 240×240 frame
  is 112.5 KiB, 240×320 is 150 KiB — both larger than what should ever be staged in
  internal SRAM next to the network stacks, and both unnecessary.
- **Draw buffers live in internal, DMA-capable SRAM** — typically the panel width × 10–40
  lines × 2 bytes (≈5–19 KB at 240 wide), double-buffered, so one buffer DMA-flushes while
  LVGL renders the next. **PSRAM is asset space, not flush space**: flush buffers placed in
  PSRAM are a documented stall trap on SPI display stacks (PSRAM is cache-mediated and not
  where SPI DMA wants to read from); decoded images and asset caches belong there instead,
  where octal PSRAM on the S3 (80 MHz DDR) is comfortable and even the first target's 2 MB
  of quad PSRAM holds a dozen full frames' worth of assets.
- **SPI wire time is the refresh ceiling, and it is high enough not to matter.** A full
  240×240 RGB565 frame is 921,600 bits: ≈23 ms at a 40 MHz SPI clock (≈43 fps theoretical)
  and ≈12 ms at 80 MHz (≈86 fps). Two routing facts gate the top rate: pins routed through
  the GPIO matrix cap at ~40 MHz, while the IO_MUX pins reach 80 MHz — the recommended
  first target (§7) routes its panel on IO_MUX pins and is vendor-rated at 80 MHz. But a
  glanceable presenter updates on *state change*, at single-digit Hz, flushing kilobytes —
  the ceiling buys smooth transitions, not a frame-rate obligation.
- **Power is backlight and radio, not pixels.** The S3's WiFi transmit peaks at ≈355 mA
  (datasheet, 802.11b at full power); the backlight is the steady drain; deep sleep is
  ≈7 µA. A battery presenter therefore duty-cycles the backlight, sleeps between events,
  and treats the touch controller's interrupt pin as its wake source. Refresh discipline
  (§3.4) is a power feature first and a bandwidth feature second.

## 3. Rendering path

### 3.1 The on-device stack

One stack covers every first-wave panel:

- **Panel IO and drivers:** `esp_lcd` — SPI (or I2C for SSD1306) bus with DMA, the panel
  drivers of §2.2, and the `esp_lcd_touch` drivers for the touch controllers.
- **Compositor:** LVGL 9 via Espressif's `esp_lvgl_port`, which owns the LVGL task, tick,
  and thread-safety lock, and registers the panel as an LVGL display with partial,
  double-buffered draw buffers in internal DMA RAM (§2.3). Touch registers as an LVGL
  pointer input device; the CST816S's hardware gesture ID is read alongside coordinates.
- **Scene:** the presenter builds LVGL objects from the view it receives (cards, §5) —
  never from bytes a guest supplied. This is #786's boundary rule restated for panels:
  the presenter owns rendering and encoding; the world supplies state, not pixels, on the
  primary path.
- **Image decode on-device:** JPEG is the supported photographic format —
  `espressif/esp_new_jpeg` decodes baseline JPEG directly to RGB565 with scale/clip/rotate
  in the decode pass (measured on ESP32-S3: a 320×240 frame decodes at ≈66 fps with
  ≈10 KB of constant working RAM, using the S3's SIMD instructions), and
  `espressif/esp_jpeg` (TJpgDec, with a decoder in ROM on ESP32/S3/C3/C6) is the
  smaller-footprint alternative. **PNG has no Espressif component** — posture: PNG stays a
  gateway-side format (§3.3); if an asset class ever demands on-device PNG, `pngle` (MIT)
  or `lodepng` (zlib license) are the named candidates, not a default.

### 3.2 What renders where

| Work | Place | Why |
| --- | --- | --- |
| Scene composition (cards, text, rings, layout) | **On-device** (LVGL) | It is a function of live world state and the panel's exact geometry; it is also cheap — LVGL was built for this class of chip |
| Dirty-rectangle flushes, transitions, local animation of held assets | **On-device** | The upload-once lesson of #5971 §2.1 applies verbatim: assets transmit once, the device animates locally, steady-state bandwidth is state diffs, not frames |
| JPEG decode of asset images | **On-device** (esp_new_jpeg) | Decode at panel geometry is a solved, measured path on the S3 (§3.1) |
| Format normalization, sanitization, resize, re-encode of uploaded media | **Upload time** (media plane) | Viewer-independent per device class — see §3.3 |
| Derivatives per device class (e.g. the 240×240-round JPEG, the 128×64 mono bitmap) | **Upload time**, cached by `(content hash, device class, converter version)` | Geometry is a device-class constant, known before any session exists |
| Video decode, video→frames, anything FFmpeg-class | **Gateway/host only — and not a tier** | #5971 §2.2's verdict, stronger here: the decode cost does not exist on-device at all. Animated assets are bounded frame sets or nothing |
| Full-frame raster rendering for the weakest devices | **Gateway** (fallback rung, §3.4) | A no-PSRAM or mono device may receive rendered tiles/frames instead of a scene description |

### 3.3 The placement rule, inverted on purpose

#5971 §5's rule is: *placement follows what the conversion's output depends on* — and for
terminals the answer was "the session" (probed tier, cell geometry, color depth), so
conversion sat at render time. **For this tier the same rule gives the opposite answer.**
An embedded presenter's geometry, color depth, and decoder set are fixed by its device
class and known in advance — there is no per-session probe to wait for. So nearly every
conversion is device-class-dependent only, which makes it upload-time work, paid once per
class and cached in the content-addressed media plane under a device-class key. What stays
at "render time" is only what depends on live state: composition and animation, which are
on-device by §3.2. The two tiers are one rule, not two exceptions — and an upload pipeline
for this tier must key derivatives by device class, never bake "the" embedded version of
an asset.

### 3.4 Refresh and update discipline

Inherited from #786's batching rules, restated for panels: state changes coalesce; the
presenter flushes only what changed (dirty rectangles, not frames); a changed viewport or
invalidated state forces one full repaint; nothing repaints on a timer unless the view
itself is time-shaped (a clock card). On reconnect after an outage the device takes one
full view, then resumes diffs — the same full-repaint-on-attach rule as the cell tiers.

## 4. Connectivity, identity, and the input back-channel

### 4.1 Links

- **WiFi (2.4 GHz, all chips)** is the primary link: the presenter joins the LAN and holds
  one long-lived connection (§4.2).
- **BLE (5.0 on S3/C3)** is the provisioning and fallback-control link: WiFi credential
  handoff at pairing, plus a low-bandwidth control channel. It is not a view transport.
- **USB** — native on the S3 (Serial/JTAG for development; OTG for a tethered presenter
  mode) — is the development, recovery, and wired path. The original ESP32 reaches USB
  only through a board's UART bridge, which is one more reason it is envelope, not target.

### 4.2 Two tracks to the world

The world's native transport is iroh. The survey found the honest answer is *two tracks*,
because the evidence cuts both ways:

**Track 2 — native iroh on-device (the direction of travel).** Iroh's own compatibility
documentation now lists ESP32 microcontrollers as supported full QUIC endpoints, and the
split is exactly this tier's memory line: **with PSRAM**, a device fits the relay client
and pkarr address lookup and is reachable from anywhere (the ESP32-S3 + PSRAM row reads
"Relay + address lookup"); **without PSRAM** (~512 KB internal SRAM only), the same
endpoint runs LAN-direct — reachable on the local network via a long ticket carrying its
address, or by endpoint ID over mDNS. The iroh project's ESP32 walkthrough demonstrates
the shape end-to-end. The costs are real and recorded, not waved away: the firmware is
Rust on ESP-IDF (the `std` track via esp-rs; Xtensa parts need the `esp` toolchain), which
sits across the grain from the C/LVGL display ecosystem of §3 — the two can be linked in
one ESP-IDF image, but the LVGL binding surface from Rust is the immature seam; and flash
budget matters (iroh's reduced-dependency build exists for <8 MB parts; the recommended
target carries 16 MB). Identity on this track is native and clean: the endpoint's keypair
*is* the device identity, and adoption is by endpoint ID.

**Track 1 — bridge (recommended first).** The device speaks one simple, TLS-wrapped
protocol to a bridge process on a host that already holds a world session: WebSocket via
`espressif/esp_websocket_client` (MQTT via ESP-IDF's `esp-mqtt` is the acceptable
alternative shape). The bridge is a presenter-side adapter in #786's sense, moved one hop
out: it terminates the world session, forwards view frames to the device, and forwards the
device's typed input events back as world input. It works for every device in the
envelope — including the C3 and no-PSRAM parts that track 2 leaves LAN-only — it keeps the
firmware entirely in the mature C/ESP-IDF/LVGL ecosystem, and it decouples the display
work from the Rust-toolchain question. Its cost is the hop: a host must be awake, and the
bridge holds world-side identity on the device's behalf (§4.3 scopes that trust).

**Recommendation:** build track 1 first (it is the first slice, §7); treat track 2 as the
named second track, gated on a small Rust-on-IDF spike that measures an iroh + LVGL image
on the target board. The view/input protocol between device and bridge should be shaped
so the bridge can later be deleted — i.e., the device-facing messages are the same view
frames and input events the native endpoint would carry.

### 4.3 Identity, session, and pairing

- **Pairing is a ceremony, and the screen is the out-of-band channel.** A new device
  displays a short pairing code (and its identity fingerprint) on its own panel; the owner
  confirms it in an existing presenter; only then does the device receive its credential.
  Bounded, explicit, one-time — #787's negotiation discipline applied to enrollment.
- **Credentials at rest** live in the device's NVS under ESP-IDF flash encryption, with
  Secure Boot v2 as the named hardening path on the S3. On track 2 the credential is a
  per-device key the bridge scopes to exactly the world(s) its owner adopted it into —
  the bridge must not be able to present a device into a world it was not adopted by.
  On track 1's threat model the bridge is trusted for liveness, not for authority: input
  events are the only device→world flow, and they enter as input, never as journal writes
  (presenters don't write journals — #786 rule 5).
- **Session behavior:** one session per device; reconnect replays nothing — the device
  requests a full view and resumes (§3.4). While disconnected, the last card stays up
  **marked stale with its age**: a presenter may show old state as old, never as fresh.

## 5. UI model: glanceable state and six verbs

### 5.1 The geometry is the design

Round and tiny viewports are constraints to design *from*, not rectangles to scale into:

- **Round 240×240:** the inscribed safe square is ≈170×170 px centered (240/√2). Text and
  touch targets live inside it. The corners and rim are for what circles do well — ring
  gauges, arc progress, ambient decoration — and never for words or targets. At 1.28″
  (≈188 ppi): touch targets ≥44 px (≈6 mm), headline figures ≥28 px, label lines ≥16 px.
- **Small rectangular (135×240 – 240×320):** the same card model with the full rectangle;
  the round layout is a subset, so a card designed round-first fits every rectangle in
  the envelope without redesign. This is why §7 targets the round panel first.
- **Very small (128×64 mono):** three to four text rows; one datum per screen; the card
  model degrades to a headline + label and nothing else (§5.4).

### 5.2 The unit is the card

One screen shows **one glanceable state**: a value, a status, a ring, a short list of at
most three rows. Cards form a stack; the world orders it; the device's whole job is the
current card, rendered well. Anything that needs reading, typing, or choosing among many
things is not card-shaped and does not belong on this tier — the card for it says so and
hands off ("continue on phone" is a first-class card state, not an error).

### 5.3 The touch vocabulary is six verbs

Bounded by the cheapest touch hardware in the envelope (CST816S: one point, gestures
recognized in the controller itself), and deliberately not extended by richer controllers
in v1:

| Verb | Gesture (CST816S gesture ID) | Meaning |
| --- | --- | --- |
| Select / ack | Tap (single click) | Primary action on the card; acknowledges an alert takeover |
| Secondary | Double-tap | The card's second action, if it declares one |
| Back / context | Long-press | Leave the card / open its context — the only "navigation chrome" verb |
| Next / previous card | Swipe left / right | Move in the card stack |
| More / less | Swipe up / down | Scroll within a card, or step its value |

Alerts are full-screen takeovers (backlight up, §2.3) dismissed by tap. There is **no text
entry, no pinch, no drag, no keyboard** on this tier — input that needs them hands off to
a full presenter (§5.2). FT6x36's second touch point buys nothing in v1 and is ignored;
a vocabulary that only the expensive panels can speak would fork the tier.

### 5.4 Display-only and ambient variants

Panels without touch (SSD1306 class; most ST7789 modules) present the same cards
read-only: the stack rotates on state change, and a single hardware button, where the
board has one, may carry tap/long-press only. Idle presenters fall to an **ambient card**
(dimmed backlight, on-change refresh — a clock, a resting status), which is the tier's
idle state and its power posture (§2.3), not a screensaver.

## 6. Licensing posture

Method, following #5971 §6: a component is *linkable* when its license permits bundling
into shipped firmware (with attribution where the license asks), *oracle/external* when it
may inform or run on a developer's host but must never be bundled or linked, and *excluded*
when the project should not touch it. Firmware has one wrinkle executables do not: the
whole image ships as a single artifact, so the rule is applied to the **component manifest**
(`idf_component.yml` + ESP-IDF's own components), and that manifest is what feeds the
license checker (#5985).

### 6.1 Linkable set — the recommended stack

| Component | What it is | License | Usable as |
| --- | --- | --- | --- |
| ESP-IDF (incl. `esp_lcd`, in-tree ST7789/SSD1306 drivers, `esp-mqtt`, lwIP, FreeRTOS port) | The framework itself | Apache-2.0 (FreeRTOS kernel MIT; lwIP BSD-style) | Framework + drivers + network |
| `espressif/esp_lcd_gc9a01` | GC9A01 round-panel driver | Apache-2.0 | Round panel driver |
| `espressif/esp_lcd_touch_cst816s` | CST816S touch driver (ESP-BSP) | Apache-2.0 | Touch, round + small panels |
| `espressif/esp_lcd_touch_ft5x06` | FT5x06/FT6x36-family touch driver (FT6236 named) | Apache-2.0 | Touch, rectangular panels |
| `espressif/esp_lvgl_port` | LVGL↔esp_lcd integration | Apache-2.0 | Compositor glue |
| LVGL | The UI library | MIT | Scene rendering |
| `espressif/esp_new_jpeg` | Baseline JPEG decoder (SIMD on S3) | Apache-2.0 | On-device image decode |
| `espressif/esp_jpeg` (TJpgDec) | Small JPEG decoder (ROM option) | Apache-2.0 component; TJpgDec core under ChaN's permissive custom license | Alternative decoder |
| `espressif/esp_websocket_client` | WebSocket client (bridge track) | Apache-2.0 | Device↔bridge transport |
| mbedTLS | TLS for WSS / HTTPS | Dual Apache-2.0 / GPL-2.0-or-later — **the Apache-2.0 option is the one taken** | TLS |
| `pngle` / `lodepng` | PNG decoders (candidates only, §3.1) | MIT / zlib | Only if an asset class demands on-device PNG |

**Verdict: the recommended stack adds no new license-checker cases.** Everything in the
linkable set is Apache-2.0, MIT, BSD-style, or a permissive custom license of the kind
#5971 already admitted (TJpgDec sits beside libjpeg-turbo's custom license in that
respect). Nothing copyleft is linked, and nothing in the set blocks bundling.

### 6.2 Obligations, external-only, and the registry long tail

| Component | License | Posture |
| --- | --- | --- |
| Arduino core for ESP32 (+ Arduino-shaped libraries: TFT_eSPI, LovyanGFX) | Core LGPL-2.1; libraries BSD-style custom licenses plus font terms | **Not used.** The native ESP-IDF path needs nothing from the Arduino ecosystem; reaching for it would import the tier's only copyleft-adjacent surface for zero capability |
| esptool | GPL-2.0 | Host-side flashing tool, run by the developer — external only, never bundled (the icat pattern of #5971 §6.2) |
| Community registry components | Mixed — the registry's long tail includes Custom, unspecified, and copyleft licenses alongside permissive ones | **Rule:** firmware pins `espressif/*` components or individually license-verified ones only; an unverified registry component is a review item, not a dependency. The manifest check belongs in #5985's checker |
| Waveshare board support / example code | Vendor examples, license varies by repo | Reference for pin maps and init sequences (behavioral oracle); firmware is written against ESP-IDF APIs, not copied from vendor examples |

## 7. Recommendation: first target and first slice

**First hardware target: the Waveshare ESP32-S3-Touch-LCD-1.28.** ESP32-S3R2 (2 MB PSRAM,
16 MB flash), GC9A01A round 240×240 panel over SPI on IO_MUX pins (vendor-rated 80 MHz),
CST816S touch, QMI8658 IMU, LiPo charge management, USB-C. Rationale, in order:

1. **It is the hardest viewport in the envelope.** Round + small + touch forces the
   safe-square discipline (§5.1) on day one; every other first-wave panel is a relaxation
   of it. A rectangle-first target would let the layout debt form silently.
2. **Every driver is an official Espressif component** (§6.1) — panel, touch, compositor,
   decode, transport — so slice 1 spends its effort on the presenter, not on drivers.
3. **PSRAM keeps both connectivity tracks open** (§4.2): it is in iroh's relay-capable
   class, so the track-2 spike can run on the same board the bridge slice ships on.
4. **It is cheap, stocked, and self-contained** (battery header + charging on board) —
   the ambient-presenter form factor is testable without a bench rig.

One recorded caveat: the board's USB-C is a CH343 UART bridge, not the S3's native
USB-Serial/JTAG — development flashing is unaffected, but the tethered-USB presenter mode
of §4.1 is validated on a plain S3 devkit, not on this board.

**Envelope checks (later slices, deliberately not first):** an ST7789 240×320 module on an
S3 devkit (rectangle sanity + display-only rotation), and an SSD1306 128×64 module (the
very-small floor, §5.4).

**First build slice: #13632** — "embedded presenter slice 1: round-panel hello-world
presenter (GC9A01 + CST816S, bridge-fed glance card, touch back-channel)". ESP-IDF firmware
for the target board: panel + touch bring-up via §6.1's components, one round-safe glance
card fed by a bridge stub serving a recorded view fixture, and the six-verb touch
vocabulary (§5.3) returned to the bridge log as typed input events. Filed with its scope,
acceptance (including on-hardware evidence), and non-goals in its body; per the operator's
standing pattern it is **not enqueued** — the reason is recorded in the issue body
(operator-managed shortlist, the #13342 pattern), and it additionally waits on the physical
board.

## 8. Open questions

- **The glanceable view contract.** What projection of world state a card stack is —
  its wire shape, ordering authority, and staleness marking (§4.3) — is the next design
  record in this line; slice 1's fixture exists to force its questions concretely.
- **Native iroh image size with LVGL in one firmware.** Iroh's reduced build targets
  <8 MB flash; the target's 16 MB should fit both, but "should" is measured by the
  track-2 spike, not assumed (§4.2).
- **PNG on-device.** Deliberately unanswered (§3.1): no asset class has demanded it yet.
- **Battery policy.** Backlight duty cycle, sleep latency vs. alert latency, and charge
  behavior for untethered presenters — measured on the first board, then designed.
- **The P4 tier.** MIPI-DSI panels and hardware JPEG change §2–§3's numbers wholesale;
  revisit only when a >320×240 target is actually wanted.

## Sources

In-repo records:

- #5971 research: `doc/2026-11-23T18-32-12Z_research_id5971_image_video_conversion_tooling/`
  (conversion findings, placement rule, licensing method; ESP-IDF scoped out in its §1 and
  taken up here).
- #786 design: `doc/2026-11-01_design_id786.md` (presenter boundary, encoding ownership,
  batching discipline). #787 (input negotiation model). #6011 (rendering ladder, closed).
  Content-addressed media plane: `doc/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane/`.
  License checker: #5985. Terminal first slice: #13623.

Web sources (surveyed 2026-11-23):

- ESP-IDF Programming Guide, LCD chapter (esp_lcd, in-tree ST7789/SSD1306):
  https://docs.espressif.com/projects/esp-idf/en/v5.2.3/esp32/api-reference/peripherals/lcd.html
- Espressif Component Registry (driver licenses, GC9A01/FT5x06/websocket client entries):
  https://components.espressif.com/components?page=3&sort_by=relevance&sort_dir=asc&q=esp_lcd ·
  https://components.espressif.com/components/espressif/esp_new_jpeg/versions/1.0.2/readme ·
  https://components.espressif.com/components/espressif/esp_jpeg/versions/1.3.2/readme ·
  https://components.espressif.com/components/espressif/esp_websocket_client/versions/0.0.3?language=en
- Espressif developer blog, esp_new_jpeg introduction (S3 decode figures):
  https://developer.espressif.com/blog/2025/09/esp-new-jpeg-introduction/
- DisplayEngineer guides (GC9A01 frame-rate/SPI math; CST816S gesture register):
  https://www.displayengineer.com/guides/round-display-gc9a01-esp32 ·
  https://www.displayengineer.com/driver-ic/cst816s
- Waveshare ESP32-S3-Touch-LCD-1.28 documentation and RIOT board page (board composition,
  ESP32-S3R2, CH343):
  https://docs.waveshare.com/ESP32-S3-Touch-LCD-1.28 ·
  https://doc.riot-os.org/boards/waveshare-esp32s3-touch-lcd-128/
- ESP32-S3 platform facts (SRAM, BLE 5.0, USB, module options):
  https://docs.zephyrproject.org/latest/boards/waveshare/esp32s3_rlcd_4_2/doc/index.html ·
  https://esp32.iammanis.com/variants/esp32-s3/
- Iroh on ESP32 (compatibility matrix — PSRAM/relay vs LAN-only; project walkthrough):
  https://github.com/n0-computer/docs.iroh.computer/blob/HEAD/compatibility.mdx ·
  https://www.iroh.computer/blog/iroh-on-esp32
- SPI PSRAM flush-buffer pitfall (third-party firmware field report):
  https://github.com/isaacarbone/WatcherOS/blob/HEAD/README.md
