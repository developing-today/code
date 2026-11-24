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

**Amendment (2026-11-23, Drewry's scope addition on #13630):** §§9–15 extend this record
with the Tanmatsu review and the app/comms/gateway model Drewry directed after the base
record landed: worlds working _as apps_ on-device (§9–§10), MQTT and a simplified HTTPS
gateway as first-class transports with embedded certificate identity (§11), capability
classes with per-class app variants (§12), live vs batch/offline operation with automatic
switching (§13), and gateway tiers reaching ESP-NOW / LoRa / Meshtastic leaves (§14). §15
states what the amendment changes in the base conclusions — and what it does not. The base
sections are unedited; where the amendment supersedes a base statement, §15 says so
explicitly.

## 1. Where this starts: the presenter model already exists

Done check against main at the time of writing (2026-11-23):

- **The boundary is already drawn.** The #786 design (`doc/2026-11-01_design_id786.md`)
  fixes the presenter contract this tier inherits: the presenter is the adapter that owns
  the output device; it probes capability and keeps the resulting profile _local_; tier
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
- **One thing this record deliberately does not design:** the world-side _view contract_
  for glanceable cards. The terminal presenters consume a cell-grid surface
  (`WorldSurface`); a 240×240 round panel needs a state-card projection instead. §5 fixes
  what the device renders and returns; the projection's wire shape is named as an open
  question (§8) for the slice line, not invented here.

## 2. Hardware envelope

### 2.1 Chips

| Chip             | CPU                                                        | SRAM   | PSRAM (typical modules)                                          | Radios                                                            | USB                                         | Place in this tier                                                                                                                                     |
| ---------------- | ---------------------------------------------------------- | ------ | ---------------------------------------------------------------- | ----------------------------------------------------------------- | ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| ESP32-S3         | Xtensa LX7 dual-core @ 240 MHz, vector (SIMD) instructions | 512 KB | 2–16 MB (quad or octal; N16R8 module = 16 MB flash + 8 MB octal) | WiFi 4 (2.4 GHz), BLE 5.0                                         | Native USB OTG + USB Serial/JTAG            | **The tier's reference chip** — every first-wave target below is S3                                                                                    |
| ESP32 (original) | Xtensa LX6 dual-core @ 240 MHz                             | 520 KB | 0–4 MB (WROVER modules carry PSRAM)                              | WiFi 4, BLE 4.2 + BT Classic                                      | None on-chip (boards use a USB-UART bridge) | Supported envelope: huge installed base of display boards; weaker (no SIMD for JPEG, no native USB, BLE 4.2)                                           |
| ESP32-C3         | RISC-V single-core @ 160 MHz                               | 400 KB | Usually none in-package                                          | WiFi 4, BLE 5.0                                                   | USB Serial/JTAG                             | Cost floor for very small panels; single core and no PSRAM put it in the bridge-only, LAN class (§4)                                                   |
| ESP32-P4         | RISC-V dual-core @ 400 MHz                                 | 768 KB | Up to 32 MB                                                      | **None on-chip** (WiFi via an ESP32-C6 companion over ESP-Hosted) | USB 2.0 HS OTG                              | Named, not targeted: MIPI-DSI and a hardware JPEG codec make it the future larger-panel tier (>320×240, RGB panels), at the price of a two-chip design |

Sources: ESP32-S3 datasheet facts as compiled in the Zephyr board documentation and the
ESP32-S3 variant survey (512 KB SRAM, BLE 5.0, USB OTG + Serial/JTAG, module PSRAM/flash
options); iroh's compatibility matrix for the P4's companion-radio arrangement.

### 2.2 Panels and touch controllers to support first

All first-wave panels are **controller-based** (ST7789/GC9A01/SSD1306 class): the panel
holds its frame in its own GRAM, the MCU streams regions into it, and ESP-IDF's `esp_lcd`
component is the uniform driver surface. ESP-IDF ships ST7789 and SSD1306 drivers in-tree;
the rest live in the Espressif Component Registry.

| Panel class       | Example geometry                                                                      | Driver path                                                    | Touch pairing                                                 | Notes                                                                         |
| ----------------- | ------------------------------------------------------------------------------------- | -------------------------------------------------------------- | ------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| Round             | GC9A01, 1.28″, **240×240**                                                            | `espressif/esp_lcd_gc9a01` (registry, Apache-2.0, all targets) | CST816S                                                       | The constraint-first viewport: §5 designs against it                          |
| Small rectangular | ST7789 family: 135×240 (1.14″), 240×240 (1.3″/1.54″), 240×280 (1.69″), 240×320 (2.0″) | `esp_lcd` in-tree ST7789                                       | CST816S or FT6x36 where fitted; many modules are display-only | One driver covers the whole rectangular small end                             |
| Mid rectangular   | ILI9341-class, 320×240 (2.8″)                                                         | ILI9341 driver in current ESP-IDF                              | FT6336 (FT6x36 family)                                        | The roomy end of "small"; where the Freenove/LCDWiki S3 boards sit            |
| Very small        | SSD1306 OLED, **128×64** mono                                                         | `esp_lcd` in-tree SSD1306 (I2C or SPI)                         | None (typical)                                                | The floor: 1 KB framebuffer, text/ticker class, display-only presenter (§5.4) |

Touch controllers, both I2C, both with official Espressif `esp_lcd_touch` drivers:

| Controller                                | Points  | Interface / address | Gesture support                                                                                              | Driver                                                                           |
| ----------------------------------------- | ------- | ------------------- | ------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------- |
| CST816S (Hynitron)                        | 1       | I2C, 0x15           | **In hardware**: slide up/down/left/right, single click, double click, long press (register 0x01 gesture ID) | `espressif/esp_lcd_touch_cst816s` (ESP-BSP)                                      |
| FT6236 / FT6336 (FocalTech FT6x36 family) | Up to 2 | I2C, 0x38           | Points + press/release events; gestures composed host-side                                                   | `espressif/esp_lcd_touch_ft5x06` (covers the FT5x06/FT6x36 family, FT6236 named) |

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
  glanceable presenter updates on _state change_, at single-digit Hz, flushing kilobytes —
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

| Work                                                                               | Place                                                                        | Why                                                                                                                                                         |
| ---------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Scene composition (cards, text, rings, layout)                                     | **On-device** (LVGL)                                                         | It is a function of live world state and the panel's exact geometry; it is also cheap — LVGL was built for this class of chip                               |
| Dirty-rectangle flushes, transitions, local animation of held assets               | **On-device**                                                                | The upload-once lesson of #5971 §2.1 applies verbatim: assets transmit once, the device animates locally, steady-state bandwidth is state diffs, not frames |
| JPEG decode of asset images                                                        | **On-device** (esp_new_jpeg)                                                 | Decode at panel geometry is a solved, measured path on the S3 (§3.1)                                                                                        |
| Format normalization, sanitization, resize, re-encode of uploaded media            | **Upload time** (media plane)                                                | Viewer-independent per device class — see §3.3                                                                                                              |
| Derivatives per device class (e.g. the 240×240-round JPEG, the 128×64 mono bitmap) | **Upload time**, cached by `(content hash, device class, converter version)` | Geometry is a device-class constant, known before any session exists                                                                                        |
| Video decode, video→frames, anything FFmpeg-class                                  | **Gateway/host only — and not a tier**                                       | #5971 §2.2's verdict, stronger here: the decode cost does not exist on-device at all. Animated assets are bounded frame sets or nothing                     |
| Full-frame raster rendering for the weakest devices                                | **Gateway** (fallback rung, §3.4)                                            | A no-PSRAM or mono device may receive rendered tiles/frames instead of a scene description                                                                  |

### 3.3 The placement rule, inverted on purpose

#5971 §5's rule is: _placement follows what the conversion's output depends on_ — and for
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

The world's native transport is iroh. The survey found the honest answer is _two tracks_,
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
_is_ the device identity, and adoption is by endpoint ID.

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

Round and tiny viewports are constraints to design _from_, not rectangles to scale into:

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

| Verb                 | Gesture (CST816S gesture ID) | Meaning                                                               |
| -------------------- | ---------------------------- | --------------------------------------------------------------------- |
| Select / ack         | Tap (single click)           | Primary action on the card; acknowledges an alert takeover            |
| Secondary            | Double-tap                   | The card's second action, if it declares one                          |
| Back / context       | Long-press                   | Leave the card / open its context — the only "navigation chrome" verb |
| Next / previous card | Swipe left / right           | Move in the card stack                                                |
| More / less          | Swipe up / down              | Scroll within a card, or step its value                               |

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

Method, following #5971 §6: a component is _linkable_ when its license permits bundling
into shipped firmware (with attribution where the license asks), _oracle/external_ when it
may inform or run on a developer's host but must never be bundled or linked, and _excluded_
when the project should not touch it. Firmware has one wrinkle executables do not: the
whole image ships as a single artifact, so the rule is applied to the **component manifest**
(`idf_component.yml` + ESP-IDF's own components), and that manifest is what feeds the
license checker (#5985).

### 6.1 Linkable set — the recommended stack

| Component                                                                                  | What it is                                       | License                                                                         | Usable as                                    |
| ------------------------------------------------------------------------------------------ | ------------------------------------------------ | ------------------------------------------------------------------------------- | -------------------------------------------- |
| ESP-IDF (incl. `esp_lcd`, in-tree ST7789/SSD1306 drivers, `esp-mqtt`, lwIP, FreeRTOS port) | The framework itself                             | Apache-2.0 (FreeRTOS kernel MIT; lwIP BSD-style)                                | Framework + drivers + network                |
| `espressif/esp_lcd_gc9a01`                                                                 | GC9A01 round-panel driver                        | Apache-2.0                                                                      | Round panel driver                           |
| `espressif/esp_lcd_touch_cst816s`                                                          | CST816S touch driver (ESP-BSP)                   | Apache-2.0                                                                      | Touch, round + small panels                  |
| `espressif/esp_lcd_touch_ft5x06`                                                           | FT5x06/FT6x36-family touch driver (FT6236 named) | Apache-2.0                                                                      | Touch, rectangular panels                    |
| `espressif/esp_lvgl_port`                                                                  | LVGL↔esp_lcd integration                         | Apache-2.0                                                                      | Compositor glue                              |
| LVGL                                                                                       | The UI library                                   | MIT                                                                             | Scene rendering                              |
| `espressif/esp_new_jpeg`                                                                   | Baseline JPEG decoder (SIMD on S3)               | Apache-2.0                                                                      | On-device image decode                       |
| `espressif/esp_jpeg` (TJpgDec)                                                             | Small JPEG decoder (ROM option)                  | Apache-2.0 component; TJpgDec core under ChaN's permissive custom license       | Alternative decoder                          |
| `espressif/esp_websocket_client`                                                           | WebSocket client (bridge track)                  | Apache-2.0                                                                      | Device↔bridge transport                      |
| mbedTLS                                                                                    | TLS for WSS / HTTPS                              | Dual Apache-2.0 / GPL-2.0-or-later — **the Apache-2.0 option is the one taken** | TLS                                          |
| `pngle` / `lodepng`                                                                        | PNG decoders (candidates only, §3.1)             | MIT / zlib                                                                      | Only if an asset class demands on-device PNG |

**Verdict: the recommended stack adds no new license-checker cases.** Everything in the
linkable set is Apache-2.0, MIT, BSD-style, or a permissive custom license of the kind
#5971 already admitted (TJpgDec sits beside libjpeg-turbo's custom license in that
respect). Nothing copyleft is linked, and nothing in the set blocks bundling.

### 6.2 Obligations, external-only, and the registry long tail

| Component                                                                | License                                                                                                        | Posture                                                                                                                                                                                                         |
| ------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Arduino core for ESP32 (+ Arduino-shaped libraries: TFT_eSPI, LovyanGFX) | Core LGPL-2.1; libraries BSD-style custom licenses plus font terms                                             | **Not used.** The native ESP-IDF path needs nothing from the Arduino ecosystem; reaching for it would import the tier's only copyleft-adjacent surface for zero capability                                      |
| esptool                                                                  | GPL-2.0                                                                                                        | Host-side flashing tool, run by the developer — external only, never bundled (the icat pattern of #5971 §6.2)                                                                                                   |
| Community registry components                                            | Mixed — the registry's long tail includes Custom, unspecified, and copyleft licenses alongside permissive ones | **Rule:** firmware pins `espressif/*` components or individually license-verified ones only; an unverified registry component is a review item, not a dependency. The manifest check belongs in #5985's checker |
| Waveshare board support / example code                                   | Vendor examples, license varies by repo                                                                        | Reference for pin maps and init sequences (behavioral oracle); firmware is written against ESP-IDF APIs, not copied from vendor examples                                                                        |

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

## 9. Amendment: the Tanmatsu review — a shipping app system on ESP32-class hardware

Drewry named the Tanmatsu as the reference design for making worlds work "as an app" on
this tier. The review below is of the board and of the app system its firmware uses
(Nicolai Electronics / badge.team lineage; the board began as the WHY2025 badge design and
is sold commercially). One correction to the framing this lane was given: **the Tanmatsu
is not an ESP32-S3 board.** Its application processor is an **ESP32-P4** — the chip §2.1
named as this tier's future larger-panel tier. The Tanmatsu is that tier, shipping.

### 9.1 The board

| Part                   | What it is                                                                                                            |
| ---------------------- | --------------------------------------------------------------------------------------------------------------------- |
| Application processor  | ESP32-P4 (dual RISC-V @ 400 MHz, 768 KB SRAM, **32 MB PSRAM**, 16 MB flash) — runs the launcher and apps              |
| Radio coprocessor      | ESP32-C6 module (WiFi 6, BLE 5.3, IEEE 802.15.4) over ESP-Hosted (SDIO) — the P4 has no radio of its own              |
| Management coprocessor | WCH CH32V203 (keyboard matrix + power management, an I2C peripheral to the P4; its firmware is updatable from the P4) |
| Display                | 3.97″ 800×480 MIPI-DSI (ST7701S), driven by Nicolai's `esp_lcd` MIPI-DSI component                                    |
| Input                  | 69-key QWERTY keyboard (no touchscreen)                                                                               |
| Extra radios           | LoRa module (Ai-Thinker Ra-01S/SH, 433 or 868/915 MHz) on the C6 side; microSD slot                                   |

Software is MIT-licensed; hardware is CERN-OHL-P-2.0. Both are bundling-compatible
postures under §6's method.

### 9.2 The app system: launcher + AppFS

The device boots into the **tanmatsu-launcher** firmware: an LVGL icon-grid menu with
status bar and keyboard navigation. Everything else on the device is an **app**, and the
mechanism is **AppFS** (badge.team's `badgeteam/appfs` ESP-IDF component, created by
Jeroen Domburg for the PocketSprite and carried by badge.team since):

- **An app is a standard, unmodified ESP-IDF application binary.** Apps require _no
  changes_ to run under AppFS; only the launcher links the component. A modified
  second-stage bootloader (part of the component) can boot a binary stored in the AppFS
  partition instead of an OTA slot — the app to start is selected via an RTC register,
  so "which app runs" survives the reset that starts it.
- **The AppFS partition is a pseudo-filesystem of app binaries** (partition type 0x43,
  subtype 0x3; 8 MB on the Tanmatsu). Each entry carries three metadata fields: a
  **slug** (unique, filename-safe, ≤48 bytes), a **title** (human-readable, ≤64 bytes),
  and a **version** (uint16). The slug also couples the binary to its data: an app's
  files live on the FAT partition (or SD card) under `/apps/<slug>/`, and the version
  field couples binary to data files.
- **A running app owns the device.** It takes the screen and input through the board
  support package (`badgeteam/badge-bsp`, MIT) and returns to the launcher on exit.
  There is no multitasking and no sandbox: one app, whole machine, launcher resumes
  after — the badge world's proven model, and the right grain for presenter firmware.
- **The launcher itself is ordinary firmware**: it occupies OTA partitions A/B (2 MB
  each) and updates over the air; NVS and the FAT data partition sit outside the A/B
  pair, which is why app data and identity survive both launcher updates and (see
  §10.3) full standalone replacements.

Stock P4 flash layout (16 MB), from the Tanmatsu flash-layout documentation:

| Region                                        | Size        | Holds                                   |
| --------------------------------------------- | ----------- | --------------------------------------- |
| Bootloader + partition table + NVS + OTA data | ~128 KB     | Boot chain and settings                 |
| Firmware A / B                                | 2 MB + 2 MB | The launcher (OTA-updatable)            |
| **AppFS**                                     | **8 MB**    | Installed app binaries                  |
| FAT filesystem                                | ~3.9 MB     | `/apps/<slug>/` data, icons, user files |

### 9.3 How apps get onto the device

Four routes, all landing in the same AppFS partition with the same metadata:

1. **The on-device app store.** The launcher's store app downloads the catalog over WiFi
   from badge.team's **Hatchery** and installs chosen apps directly into AppFS. The
   Hatchery carries the compatibility precedent §12 builds on: at publish time an author
   selects which devices an app is compatible with, and a device's installer only offers
   compatible apps.
2. **BadgeLink (USB).** A host protocol (WebUSB from Chromium, or a Python tool) that
   manages the device: `appfs list / usage / upload / download / delete`, `start <slug>`,
   plus NVS read/write and FAT filesystem access. This is the developer/sideload route —
   and, for this tier, the provisioning route (§11.3): anything that must be placed on
   the device (an app, a certificate, a key) can be placed over BadgeLink during the
   pairing ceremony.
3. **Host-side image tools.** `appfs_generate.py` builds an AppFS partition image and
   `appfs_add_file.py <image> <app.bin> <slug> <title> <version>` inserts apps into it
   before flashing — i.e. **apps embedded in the firmware image at build time** (§10.2).
4. **An app as main firmware.** The template project also supports installing one app
   _as_ the device firmware, replacing the launcher entirely (§10.3).

### 9.4 What this tier adopts from the Tanmatsu model

- **The presenter is an app.** The embedded presenter ships as a standard ESP-IDF app
  binary with slug/title/version metadata — installable, listable, deletable, and
  startable by whatever launcher the device runs, and equally able to _be_ the firmware
  on a dedicated device. Nothing presenter-specific is baked into a launcher.
- **Slug-keyed data.** Presenter state (pairing record, offline queue, cached assets)
  lives under the app's own data directory, keyed by slug — so the app and its data
  version and move together, as AppFS's binary↔files coupling intends.
- **BSP-shaped hardware access.** Apps touch hardware through a board support package,
  never board-specific code — the same discipline as §3's `esp_lcd` uniformity, one
  level up. A presenter app built this way ports across boards by swapping the BSP.
- **A host management protocol.** BadgeLink's shape (list/install/start apps, read/write
  settings, move files, over USB) is the provisioning and recovery surface a fleet of
  presenter devices needs; §11.3 uses it for certificate placement.
- **One honest gap:** stock AppFS bootloader support covers ESP32, ESP32-C6, and
  ESP32-P4 — **not yet the ESP32-S3**, this tier's first target (§7). Adopting AppFS
  literally on the S3 presenter waits on an upstream bootloader port (or a port carried
  as part of a later slice). On P4-class hardware it works today. The app _shape_ (§10)
  is adoptable everywhere immediately, because it asks nothing of the app itself.

## 10. Amendment: the app model — selection, embedding, and kiosk boot

### 10.1 Selection among multiple installed apps

The launcher enumerates the AppFS partition (slug, title, version) and presents it as
the menu; the user picks, the bootloader boots that app, the app runs until it exits
back to the launcher. For a presenter device the same mechanism reads differently:
**one device can carry several world apps** — a presenter per world, or a glance
presenter plus a diagnostics app plus a provisioning app — and selection is a launcher
act, not a reflash. Two presenter-specific notes:

- App switching is a reboot into the chosen app (the RTC-register selection), so it is
  a mode change, not a task switch; a presenter app must therefore persist its session
  and queue state in its data directory and resume cleanly (§13's queue lives there).
- A "default app" notion is all kiosk mode needs (§10.3): the launcher needs one
  remembered slug.

### 10.2 Embedding one or more apps in the firmware image

Because AppFS images are built host-side (§9.3, route 3), a shipped firmware image can
carry **any set of apps pre-installed** in its AppFS partition alongside the launcher:
the factory image for a presenter device is launcher + presenter app (+ provisioning
app) composed at build time, with no on-device install step. The same composition is
how a _bundle_ ships: several world presenters, or a presenter plus its fallback
variant (§12.3), embedded together and selected at runtime. Nothing about the apps
changes — embedding is a packaging act on the partition image, which is exactly the
property that makes it safe to adopt.

### 10.3 Booting straight into a single app (kiosk mode)

Two mechanisms exist in the field, with different trade-offs:

| Mechanism                                                                          | How                                                                                                                                                                                    | Cost / posture                                                                                                                                                                                                                                                                                                              |
| ---------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Standalone app image** (proven: the wadamesh Tanmatsu build offers exactly this) | The app is built/flashed as the device firmware, replacing the launcher; the device boots into it directly                                                                             | The launcher and other apps are gone (recoverable via the recovery site / reflash). Notably, the data partition is kept at the same address, so identity, contacts, and settings **survive** switching between app-mode and standalone mode — the precedent that identity must live outside the firmware partitions (§11.3) |
| **Launcher auto-start** (mechanism present, setting not shipped)                   | The bootloader already boots an RTC-register-selected app; a launcher that records a default slug and selects it on boot gives kiosk behavior while keeping the launcher one exit away | Not a documented stock Tanmatsu feature — named here as this tier's design choice for dedicated presenters, because it preserves the recovery and provisioning surface that standalone mode gives up                                                                                                                        |

Recommendation for this tier: **auto-start for dedicated presenters, standalone only
for sealed appliances.** A world presenter that can still exit to a launcher can be
re-paired, updated, and recovered in the field; a sealed one cannot.

## 11. Amendment: alternate comms — MQTT and a simplified HTTPS gateway as first-class transports

§4.2's bridge track said "WebSocket … (MQTT via ESP-IDF's `esp-mqtt` is the acceptable
alternative shape)". Drewry's direction promotes the alternatives to first-class:
the device↔bridge protocol gets **three coequal transports**, chosen per deployment,
all carrying the same view frames and typed input events (the §4.2 rule that the bridge
must be deletable later applies to all three):

| Transport                         | Shape                                                                                                  | When it is the right choice                                                                                                                                                               |
| --------------------------------- | ------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| WebSocket (base record)           | One long-lived TLS connection, frames both ways                                                        | LAN presenters with a host bridge awake; lowest latency                                                                                                                                   |
| **MQTT** (`esp-mqtt`, in ESP-IDF) | Publish/subscribe via a broker the gateway hosts                                                       | Fleets and intermittently-connected devices; the broker _is_ the meeting point, so neither side needs the other awake                                                                     |
| **Simplified HTTPS gateway**      | Plain HTTPS requests: the device POSTs input events and GETs its current view from a small gateway API | The weakest devices and the nastiest networks: no long-lived connection, no broker client, works through proxies and captive-ish networks; polling cost is paid in latency and radio time |

### 11.1 MQTT maps onto the card model unusually well

- **The current view is a retained message.** The gateway publishes the device's
  current card stack to the device's view topic with the retain flag; a device that
  connects (or reconnects) receives its current view immediately from the broker —
  §3.4's full-view-on-attach rule, implemented by the broker instead of by session
  logic. Updates are subsequent publishes; diffs remain an optimization, not a
  correctness mechanism.
- **Input events are publishes** on the device's event topic, QoS 1 (at-least-once,
  deduplicated by the event's device sequence number, §13.2).
- **Liveness is the Last Will.** The broker publishes the device's will message when it
  drops ungracefully — which is precisely §4.3's "marked stale" state, surfaced by the
  transport instead of by a timeout the world must run. Presence is a retained topic
  for the same reason.
- **The offline queue has a natural home at both ends** (§13.3): the broker holds
  retained view + any queued commands while the device is away; the device holds its
  unacknowledged event spool while the broker is away.

### 11.2 The simplified HTTPS gateway

Deliberately the smallest API that carries the model: `GET /view` (the device's
current card stack, with a version/ETag so an unchanged view costs a 304), `POST
/events` (a batch of queued input events — the same endpoint serves live single events
and §13's batch drains), `GET /config` (mode, poll interval, staged commands). Every
call is one short TLS session with the device certificate (§11.3). This is the
transport of last resort and the batch mode's natural drain path (§13.4); its pull-only
shape is also its honesty: a device on HTTPS polling is never _pushed_ to, and §13
says what that means for alerts.

### 11.3 Embedded certificate/key: device identity for the alternate transports

WebSocket, MQTT over TLS, and HTTPS all authenticate the same way: **mutual TLS with a
per-device client certificate**, provisioned once and embedded in the device:

- **Provisioning is part of the pairing ceremony** (§4.3). At pairing, the device
  generates its keypair on-device (the private key never leaves it), shows its pairing
  code on its own screen, and — once the owner confirms — receives its certificate from
  the owner's gateway/CA, delivered over the local link (BLE provisioning channel, or
  BadgeLink-class USB on launcher devices, §9.3). What is "embedded" in firmware is
  therefore the _mechanism_ (cert/key storage slots, the CA root), never a shared
  secret: each device's key is its own.
- **Storage** follows §4.3: key and certificate in NVS/flash under flash encryption,
  in a data partition that survives firmware replacement (the Tanmatsu standalone
  precedent, §10.3: the data partition address is stable across mode switches, so the
  identity does too). ESP-IDF embeds cert/key material into the image or a partition
  via its standard embedded-file mechanism, and `esp_crt_bundle` covers server
  verification against public roots where the gateway is publicly hosted; a private
  gateway CA is pinned instead.
- **One identity across transports.** The certificate's identity _is_ the device
  identity §4.3 adopts it under — the same device presenting over WebSocket today and
  MQTT tomorrow is one device, and on track 2 (§4.2) the same keypair discipline is
  what an iroh endpoint key already is. Revocation is by the gateway refusing the
  certificate; rotation is a re-provisioning, using the still-valid old credential to
  authenticate the new one.

## 12. Amendment: capability classes and per-class app variants

The envelope of §2 is wide — a P4 palmtop and a 128×64 mono panel are not one device.
Drewry's direction: **app compatibility is declared per capability class**, and devices
run the variant built for their class. The Hatchery precedent (§9.3) is the store-side
half of this: authors declare compatibility, installers filter by it. This section
defines the class side.

### 12.1 The classes

A class is a device constant — like panel geometry in §3.3, it is known before any
session exists, so variant selection is an install-time act, never a runtime probe.

| Class           | Exemplar hardware                                       | Presenter form                                                                                                                                  |
| --------------- | ------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| **A — palmtop** | Tanmatsu (P4 + C6, 800×480, keyboard, 32 MB PSRAM)      | Full presenter app: card stacks, lists, text-capable input via keyboard; can host the gateway role (§14)                                        |
| **B — glance**  | §7's target (S3 + PSRAM, round/small touch panel)       | The §5 card presenter with the six-verb touch vocabulary                                                                                        |
| **C — reduced** | S3/original ESP32 without PSRAM, or display-only panels | Card presenter without on-device JPEG/photo cards (gateway-rendered tiles, §3.4's fallback rung); display-only rotation where there is no touch |
| **D — minimal** | C3-class, SSD1306 128×64 mono                           | Headline/ticker app (§5.4): one datum, no cards; or a leaf behind a §14 gateway                                                                 |

### 12.2 Declaring compatibility

An app's store/launcher metadata gains a **class requirement** beside AppFS's
slug/title/version: the minimum class, plus named feature requirements where a class
is not granular enough (touch, PSRAM, panel-geometry family, keyboard, LoRa). The
installer — on-device store, BadgeLink-class host tool, or factory image composer
(§10.2) — refuses or filters apps whose requirements the device does not meet, in the
Hatchery compatibility manner. A presenter app that silently degraded on a too-small
device would violate §4.3's honesty rule in a new place; declared refusal is the
correct failure.

### 12.3 Variants and alternate apps

- **Variants** are per-class builds of one app under one slug: the template project's
  `make build DEVICE=<target>` is the working precedent — one source tree, per-device
  binaries, the store/installer picking the binary that matches the device's class.
  Variant selection happens at install; the device itself never downloads a binary
  "for" another class.
- **Alternate apps** cover the cases a variant cannot: where a class changes the
  _interaction model_ and not just the rendering (class D's ticker vs class B's card
  stack; a display-only build with no input back-channel), the honest packaging is a
  separate app under its own slug, declared as the alternate for that class — so a
  fleet tool can answer "which presenter runs on this device" from metadata alone.
- **Fallback within a session** stays §3.4's ladder (scene → gateway tiles → headline);
  classes decide what an app _is_, the ladder decides how a given view is carried.
  The two must not be conflated: a class B device receiving a photo card it cannot
  decode takes the ladder's tile rung; it does not become class C.

## 13. Amendment: live vs batch/offline — dedicated modes with automatic switching

§4.3 specified disconnected behavior as a display rule (last card, marked stale). The
amendment makes offline a **mode of operation**, not an error state: a presenter can be
_deployed_ offline-first (a garden sensor's display, a badge at a camp with no
infrastructure — the badge world's normal case) as well as fall into offline when a
live link dies.

### 13.1 The two modes

|               | Live mode                                                           | Batch/offline mode                                                                                                   |
| ------------- | ------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| View          | Gateway pushes view changes as they happen (WS frame, MQTT publish) | The device renders from its last synced card stack; updates arrive at sync points                                    |
| Input         | Each typed event is sent as it happens                              | Events are **recorded to the on-device queue** and uploaded in a batch when connected                                |
| Alerts        | Full-screen takeover on arrival                                     | No push exists; an alert is shown at the next sync, or immediately if it was already staged on-device with an expiry |
| Power posture | Radio associated, connection held                                   | Radio sleeps between sync windows; touch/ button wake can force an early sync                                        |

A device is in exactly one mode at a time, and **the mode is visible**: the ambient
card (§5.4) shows live/batch state and queue depth — a user must be able to see that
their taps are being recorded for later, not delivered now. Invisible batching would
break §4.3's staleness honesty in the input direction.

### 13.2 The on-device queue

- **Where:** the app's data directory (§9.4) — a small append-only log in flash (FAT
  file on launcher devices; a dedicated data partition or NVS-backed log on bare
  presenters). Sized for days of six-verb events, which are tens of bytes each; the
  queue is events and small state notes, never media.
- **Entry shape:** `(device sequence number, device timestamp, verb, card context)`.
  The sequence number is the deduplication key (§11.1's QoS-1 duplicates are expected,
  not an error); the timestamp is device time, recorded as such — on upload the
  gateway stamps arrival time alongside, and the world sees both. Batched input enters
  the world as input events _with their age attached_; presenters still never write
  journals (§4.3), and a world that cannot accept aged input for a given card says so
  in that card's contract (the view contract of §8 gains this question).
- **Drain semantics:** on connection, the device uploads in sequence order and deletes
  only acknowledged prefixes; a half-drained queue resumes where it stopped. The
  gateway acknowledges by sequence number, so a crash on either side costs a re-send,
  never a loss or a silent double-apply.

### 13.3 Push and pull, in both modes

| Direction      | Live                                                                        | Batch/offline                                                                                                                                                                              |
| -------------- | --------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| World → device | **Push**: WS frame / MQTT publish (retained view topic keeps it idempotent) | **Staged pull**: the gateway holds the new view (retained MQTT message, or the HTTPS gateway's current-view document); the device pulls it at its next sync and takes the full view (§3.4) |
| Device → world | **Push**: event sent on occurrence                                          | **Push-at-sync**: the queue drains as one batch upload (MQTT publishes or one HTTPS `POST /events`) the moment connectivity returns                                                        |
| Wake           | n/a (connection held)                                                       | The sync trigger itself: timer, touch/button wake, or a §14 gateway's wake packet on the local link (ESP-NOW/BLE) telling the leaf a staged view exists                                    |

Pull is also the live mode's recovery: any reconnect, in any mode, begins with the
device pulling its current view rather than trusting what it held (§3.4).

### 13.4 Automatic switching

Mode is a device-side policy with three inputs, in priority order: **(1) explicit**
— a mode verb from the owner (a card action, a launcher setting, a gateway command in
`/config`); **(2) power** — a battery device below its charge threshold drops to batch
and lengthens sync intervals, because the radio is the power budget (§2.3); **(3)
link reality** — _N_ consecutive failed heartbeats/connect attempts (or an MQTT will
from its own gateway session ending) switches live → batch, and the first successful
sync switches batch → live, with hysteresis (a minimum dwell in each mode) so a
flapping link produces one switch, not fifty. Every switch is itself queued as a
state note, so the world learns the device was batching — and for how long — from the
drain, not from a gap it must guess at.

## 14. Amendment: gateway tiers — stronger boxes proxy, connected ESPs funnel

§4.2's bridge was a host process. The amendment generalizes it into **two gateway
tiers**, because Drewry's topology has devices that will never see a LAN: a presenter
in range of another ESP but not of any infrastructure.

### 14.1 The tiers

| Tier                  | Hardware                                                                                                                             | World side                                                                                                                              | Device side                                                                                                                                              |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **G2 — full gateway** | Laptop/host/phone-class box (or a class A palmtop, §12)                                                                              | A **real iroh endpoint** holding world sessions (this is §4.2 track 2, terminated on a box that can afford it) plus the bridge          | Hosts the MQTT broker and/or HTTPS gateway (§11) and WebSocket bridge for LAN devices; holds per-device staged views and the durable copy of every queue |
| **G1 — ESP funnel**   | An ESP with connectivity (S3 on WiFi; a Tanmatsu-class device whose C6 gives it WiFi/BLE/802.15.4 and whose LoRa module is on board) | One upstream connection as a _client_: MQTT or HTTPS to a G2 (or directly to a broker) — a funnel ESP needs no world session of its own | Southbound links to leaves: **ESP-NOW**, **BLE**, **LoRa** (raw or Meshtastic, §14.3)                                                                    |

The rule that makes the tiers compose: **a gateway is a client upstream and a bridge
downstream, and it holds no world authority of its own.** A leaf's input events travel
leaf → G1 → G2 → world as input events at every hop (§4.3's rule, extended: every
gateway is trusted for liveness, none for authority), and the G2's iroh endpoint
identity is the _gateway's_, adopted for the worlds it serves — the devices behind it
are adopted individually (§14.2), not by transitivity.

### 14.2 What a far (leaf) device needs

A leaf is a device whose only link is a southbound one (ESP-NOW/BLE/LoRa) to a G1:

- **Its own identity.** A per-device keypair/certificate as in §11.3, provisioned at
  pairing _through_ the gateway (the ceremony's confirm step happens on whatever
  presenter the owner has; the credential reaches the leaf over its local link during
  the ceremony window). Where the leaf is too small for TLS (class D), the floor is a
  provisioned per-device symmetric key plus device ID, used by the link-layer security
  below — named as a floor, with its weaker revocation story stated, not hidden.
- **A registered place at its gateway:** an ESP-NOW peer entry (MAC + per-peer key),
  a LoRa/Meshtastic address, or a BLE pairing — plus the gateway's record of which
  device identity that link address speaks for. Link addresses are routing facts, not
  identities; the gateway's mapping is what §11.3's certificate authenticates when the
  leaf's traffic reaches G2.
- **Almost nothing else.** No IP stack, no TLS requirement on the far leg, no world
  knowledge. A class D leaf needs: its key, its gateway's link address, the card/headline
  rendering of §5.4, and the §13.2 queue discipline in miniature (below).

### 14.3 The southbound links, honestly sized

| Link                                      | Envelope                                                                                                                                         | Consequences for the model                                                                                                                                                                                                                                                                                                                                                                           |
| ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **ESP-NOW** (ESP-IDF, Apache-2.0)         | 2.4 GHz, ~200 m line-of-sight, ≤250-byte packets, ≤20 peers (10 encrypted), per-peer AES-128 (CCMP), **no routing** — direct range only          | View frames must be chunked with reassembly, or reduced to headline form for the far leg; forwarding beyond one hop is application-level (a G1 relaying to another G1), designed, not assumed                                                                                                                                                                                                        |
| **LoRa, raw**                             | Kilometres at kbps-and-below, small packets, duty-cycle limits in some bands                                                                     | Only headline/ticker traffic and batched event drains fit; a photo card does not cross a LoRa leg in any form — §12's classes and §3.4's ladder both terminate at "headline" here                                                                                                                                                                                                                    |
| **Meshtastic** (as a network to speak to) | LoRa mesh firmware with flood routing (hop limit 3 default, 7 max), store-and-forward facilities, per-channel keys + public-key DMs; **GPL-3.0** | **Interop only, never bundled** (§6's method): our gateway runs its own firmware and speaks Meshtastic's protocol over the air or to a Meshtastic radio over serial — the Tanmatsu's own Meshtastic arrangement is the precedent (GUI app on the P4, Meshtastic firmware on the C6 radio). Where a mesh stack must be _bundled_ into gateway firmware, MeshCore (MIT) is the named candidate instead |

### 14.4 Where the offline queue lives in this topology

**At the first upstream node with durable storage and power — and mirrored at the
device when it has storage of its own.** Concretely:

- A leaf with flash (any class) keeps its §13.2 unacknowledged-outbox until its
  gateway acknowledges receipt; the outbox is small because acks are one hop away.
- **The gateway holds the durable queue**: per-leaf staged views waiting to go down,
  and per-leaf event batches waiting to go up, in the gateway's flash (G1) or disk
  (G2). If the G1→G2 leg is down, the G1 _is_ the offline queue for its whole subtree —
  which is §13's batch mode with the batch point moved up one tier, and the same
  drain/ack discipline applied recursively.
- A storage-less leaf (below class D, if such a presenter is ever built) holds nothing:
  its gateway's per-leaf mailbox is its only queue, and its input is live-or-lost — a
  posture to state on its class declaration (§12.2), never to discover in the field.

## 15. Amendment: what changes in the base conclusions — and what does not

- **§7's first target and first slice stand.** The Waveshare S3 round panel remains the
  hardest-viewport-first target, and #13632's scope is unchanged: bridge-fed glance
  card, six verbs, fixture. Felix is noting on #13632 the one build-shape consequence
  of §9.4: slice 1's firmware is written as a standard installable app binary with
  slug/title metadata from the start (asking nothing of the app is AppFS's whole
  point), so it can drop into an AppFS partition or a minimal launcher later without
  rework — and literal AppFS adoption on the S3 waits on the §9.4 bootloader gap.
- **The P4 is promoted from "named, not targeted" (§2.1) to a named second hardware
  track**: the Tanmatsu class (class A, §12.1) is now a target of the program, with
  its own first slice filed from this amendment — **#13643**, the world presenter as
  an AppFS app on Tanmatsu-class hardware. It is not enqueued (operator-managed
  shortlist, the #13342 pattern; it also waits on a physical Tanmatsu on the desk).
- **§4.2's transport paragraph is extended, not replaced**: WebSocket remains a
  first-class transport; MQTT and the simplified HTTPS gateway join it as coequal
  (§11), and §13's mode machinery is the connective tissue between all three.
- **§6 gains rows, and one honest correction of emphasis.** The amendment's stack —
  `badgeteam/appfs` (Beer-Ware for the component; Apache-2.0 for the bootloader
  overrides), `badgeteam/badge-bsp` (MIT), the tanmatsu-launcher (MIT), ESP-NOW and
  `esp-mqtt` (ESP-IDF, Apache-2.0), MeshCore (MIT, candidate) — is permissive
  throughout, **with exactly one new license-checker case**: Beer-Ware is a permissive
  custom license of the retain-notice kind §6 already admits (TJpgDec's class), and
  #5985's checker must know it by name. Two exclusions by §6's own method: Meshtastic
  (GPL-3.0) is interop/external only, and painlessMesh (GPL-3.0) is not used. One
  provenance caveat, recorded because a field guide flagged it: the AppFS _host tools'_
  circulating copies carry no license header — the component itself is licensed as
  above, but the tier pins the registry component (`badgeteam/appfs`), not a copied
  script.
- **New open questions** (joining §8): the AppFS bootloader port for ESP32-S3 (upstream
  gap; porting it is a slice candidate, not an assumption); whether the launcher
  auto-start setting (§10.3) should be contributed upstream to the tanmatsu-launcher
  or carried in this tier's own minimal launcher; the view contract's treatment of
  aged batch input (§13.2); and the far-leg fragmentation format for view frames over
  250-byte links (§14.3) — headline-only first, chunking only if a real deployment
  asks for more.

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

Web sources for the amendment (§§9–15, surveyed 2026-11-23):

- Tanmatsu documentation (AppFS mechanics and metadata limits; BadgeLink app/NVS/FS
  management; P4 flash layout; Meshtastic arrangement — GUI app on the P4, radio
  firmware on the C6; template app build incl. `DEVICE=` targets and install-as-firmware):
  https://docs.tanmatsu.cloud/software/appfs/ ·
  https://docs.tanmatsu.cloud/software/badgelink/ ·
  https://docs.tanmatsu.cloud/software/flash_layout/ ·
  https://docs.tanmatsu.cloud/software/meshtastic/ ·
  https://docs.tanmatsu.cloud/software/compiling_the_template_app/ ·
  https://docs.tanmatsu.cloud/hardware/specifications/
- AppFS component, registry entry (component Beer-Ware license; bootloader overrides
  Apache-2.0) and release notes (RTC-register app selection):
  https://components.espressif.com/components/badgeteam/appfs/versions/1.0.2/readme ·
  https://github.com/badgeteam/esp32-component-appfs/releases/tag/v1.0.2
- badge-bsp license (MIT):
  https://components.espressif.com/components/badgeteam/badge-bsp/versions/2.0.2/license
- Tanmatsu hardware composition and licensing posture (P4 + C6 + CH32, LoRa module,
  800×480 ST7701S, software MIT / hardware CERN-OHL):
  https://hackaday.com/2026/07/01/review-the-tanmatsu-a-year-on/ ·
  https://www.hackster.io/news/renze-nicolai-s-tanmatsu-is-a-powerful-radio-packed-espressif-esp32-p4-pocket-terminal-b26b16524a76
- A shipping app's Tanmatsu practice (AppFS sideload discipline; standalone build that
  replaces the launcher while preserving the data partition; app-store install as the
  recommended route):
  https://www.wadamesh.com/tanmatsu.html ·
  https://git.meshinfo.ru/shade/wadamesh/src/commit/82238d3f7c1d7c20910986d4fee22d5d8cf706f3/TANMATSU_SIDELOAD.md
- Hatchery compatibility practice (authors select compatible devices at publish;
  on-badge installer offers only compatible apps):
  https://badge.team/docs/badges/campzone-2019/ ·
  https://pixel.curious.supplies/docs/app-development/publishing/
- ESP-NOW envelope (250-byte packets, peer limits, per-peer AES, no routing) and the
  ESP mesh landscape (Meshtastic GPL-3.0 / MeshCore MIT; ESP-WIFI-MESH and ESP-NOW
  Apache-2.0):
  https://www.mesh101.com/news/post-24-esp-now/ ·
  https://www.offlineprotocol.com/learning/embedded/esp32-mesh-network-options
