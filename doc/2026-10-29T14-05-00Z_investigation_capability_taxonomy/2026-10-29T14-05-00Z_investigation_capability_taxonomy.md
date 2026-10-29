# Capability taxonomy v0 — interface specialization across web, ESP32/MQTT and text/audio

Investigation for [#2027](https://github.com/developing-today/code/issues/2027) (wave-1 triage task 3344, verdict CONFIRMED-with-refinement). Written 2026-10-29 against `main` at `5a34fa70`.

Status: **v0, investigation only.** Nothing here is built by this record. Every buildable recommendation has a follow-up issue, linked from #2027; a taxonomy with no implementation link would not be done.

## 1. What exists, and what the taxonomy has to extend

The issue assumed interface capabilities were greenfield. They are not entirely: two adjacent layers exist and this taxonomy is defined *as an extension of them*, deliberately not as a parallel capability system.

### 1.1 `pkgs/id/src/world_caps.rs` — a request/response model, for server data

A world program stays a pure function. It returns a `wants` document (v1) which is a pure projection of its model:

```json
{"v":1,
 "subscribe":["players","time.tick:5000"],
 "requests":[{"id":"r1","cap":"time.now"},
             {"id":"r2","cap":"chat.say","args":{"text":"hello"}}]}
```

The server answers each new request with a `cap: result` event — `{"cap":"result","id":"r1","ok":true,"value":...}` or `"ok":false` with a stable `"error"` code plus an advisory `"reason"` — and each subscription with its own events. Properties worth reusing verbatim:

| Property | Current value | Reusable for interface caps? |
| --- | --- | --- |
| Request shape | `{id, cap, args}` | Yes, unchanged |
| Response shape | `{"cap":"result","id",ok,value\|error}` | Yes, unchanged |
| Refusal codes | `cap_denied`, `unknown_cap`, `bad_args`, `failed`, `actor_denied`, `outside_ceiling` | Yes, all six cover device refusal; only the advisory `reason` needs new vocabulary |
| Catalog | `CATALOG`, 17 names, "new providers add names; existing names never change meaning" | Yes — extend by adding names, no version bump |
| Bounds | wants 64 KiB, ≤64 entries, name ≤64 B, args ≤8 KiB, event ≤64 KiB, ≤4 tick intervals | Per new cap, not per protocol |
| Grants | policy + ceiling, grant-on-use, usage accounting (`CapUsage`) | Yes for server-side caps; see §4 seam for device/sensor caps |
| Trust | local to the hosting node, journaled, **not signed, not replicated** | Yes — a device manifest is equally node-local |
| Version | `WANTS_VERSION = 1` | Yes — v1 already admits new names, so interface caps need no schema bump |

The 17 catalog names are `time.now`, `time.tick`, `random.u64`, `players`, `players.list`, `chat.say`, `chat.history`, `records.read`, `schedule.at`, `schedule.cancel`, `world.info`, plus the Roc-platform presentation names `Key`, `Screen`, `RocJson`, `RocGrid`, `RocCap`, `RocText`. **No `form.*`, `loc.*` or `media.*` name exists**, and `grep -rln 'esp32\|mqtt' pkgs/id/src/` returns nothing at HEAD.

### 1.2 `pkgs/id/src/world_presentation.rs` — one surface kind

`WorldSurface` is a tagged enum (`#[serde(tag = "kind", rename_all = "snake_case")]`) with exactly one variant:

- `Screen(ScreenSurface)` — `{version: 1, width, height, cells}` row-major, ≤120×50, ≤4096 coalesced style spans, plus `ScreenPatch` (`diff`/`apply`) and `fit()` for connection-local viewport cropping.

Two properties of this module carry straight into the interface taxonomy:

1. **Guests never supply terminal escapes, HTML or CSS.** Adapters render the validated value with their own trusted code. Any interface capability must keep that: a program may *name* an affordance, never *implement* one.
2. **Legacy UTF-8 text remains valid.** `parse_surface` returns `Ok(None)` for non-screen documents, so every program already has a text rendering path. That is the hook the text-only mode in §7 formalises rather than invents.

## 2. The taxonomy: three axes, not one

The issue asks for one taxonomy across form, location, media, MQTT and audio. One axis cannot hold them, because they differ in *who* grants them and *what fails* when refused. v0 separates three:

- **Axis A — subject of the capability.** What the program is asking for: server data (existing), a device affordance (new), a sensor reading (new), a media artifact (new).
- **Axis B — interaction shape.** One-shot request/result, subscription (a stream of events), or artifact reference (a handle to bytes held elsewhere). These map onto the existing `requests` / `subscribe` / returned-value mechanics with no wire change.
- **Axis C — grant authority.** Server owner (world grant policy), device (hardware/ceiling), or the human at the device (consent). Every new cap must name exactly one authority; caps that need two are split, not merged.

### 2.1 Namespaces (v0 proposal)

| Namespace | Axis A | Examples | Shape | Grant authority |
| --- | --- | --- | --- | --- |
| `device.*` | device affordance (description) | `device.info` | one-shot | device |
| `form.*` | device affordance (input/output) | `form.present`, `form.result`, `form.cancel` | request + later result | device + world ceiling |
| `loc.*` | sensor | `loc.position`, `loc.watch` | one-shot / subscription | human (consent), device (accuracy ceiling) |
| `media.*` | media | `media.record`, `media.play`, `media.image`, `media.audio`, `media.stream` | request / subscription / artifact ref | human + device |
| `speech.*` | media (audio, conversational) | `speech.transcribe`, `speech.capture`, `speech.synthesize` | one-shot / subscription | human (per app) |
| `text.*` | presentation mode | none required — a *mode*, not a cap (see §7) | — | world |
| `surface.*` | presentation (reserved) | reserved so that `Screen`'s successors share a prefix | — | device |

Naming rules carried over: flat dotted ASCII, ≤64 bytes, never reused, meaning never changes. Lower-case namespaces for new caps; the existing capitalised (`Key`, `Screen`, `Roc*`) names are Roc platform parity names and stay as they are — they are not a style to copy.

### 2.2 Refusal vocabulary: reuse codes, extend reasons

The six codes stay fixed. What differs per platform is *why*, and that is exactly what the advisory `"reason"` string is for. v0 assigns reasons (codes unchanged, so the DoS suite's assertions still hold):

| Situation | Code | Advisory reason |
| --- | --- | --- |
| No consent for mic/camera/location | `cap_denied` | `consent_required` |
| Consent given but OS/permission layer refused | `cap_denied` | `os_permission_denied` |
| Device has no such hardware (no GNSS, no camera) | `unknown_cap` | `device_lacks_affordance` |
| Device accuracy/precision below the request | `outside_ceiling` | `precision_ceiling` |
| Location fix unavailable (indoors, no sky view) | `failed` | `no_fix` |
| Media larger than the artifact ceiling | `bad_args` | `oversize` |
| Form field value rejected by the program's schema | `bad_args` | `field_invalid` |

This keeps one wire contract across web, ESP32 and text while letting a device say something a server cannot.

## 3. Per-capability detail

### 3.1 `device.info` — the root of adaptation

A one-shot request returning the device's **capability manifest**, node-local and journaled like every other grant:

```json
{"cap":"result","id":"d1","ok":true,"value":{
  "manifest":{"v":1},
  "display":{"kind":"pixel","width":128,"height":64,"colors":1},
  "input":{"keys":8,"encoder":true,"pointer":false,"text_entry":"none"},
  "sensors":{"gnss":{"accuracy_m":5},"microphone":false,"camera":false},
  "audio":{"out":"pcm","in":null},
  "network":{"transport":"mqtt","rtt_class":"slow"}
}}
```

Why it is first: everything else in this taxonomy is conditional on the device's affordances, and today a program has no way to ask. It is deliberately a *capability*, not a field on the surface, so that a program can make the same model render three ways (§8) without the server knowing which device it is.

### 3.2 `form.*` — structured input, not HTML

`form.present` carries a small declarative field set (`text`, `choice`, `number`, `date`, `confirm`) plus actions; the device renders it with its own trusted widgets; the answer arrives as a `form.result` event `{form_id, action, values}`. A `form.cancel` event carries user abandonment. No program-supplied HTML, CSS or validation code — validation is the program's job on the result, and a rejected value is `bad_args` + `field_invalid` for a re-present, not a server-side retry loop.

Ceiling: field count and total payload share `MAX_ARGS_BYTES`; the response obeys `MAX_EVENT_BYTES`.

### 3.3 `loc.*` — location with an accuracy ceiling

`loc.position` (one-shot fix), `loc.watch` (subscription with an interval floor). Every reading carries `{lat, lon, accuracy_m, fix_age_ms, source}`; `source` is `gnss | network | ip | caller_supplied`. The **precision ceiling** is the device's `outside_ceiling`: an ESP32 with GNSS can serve `accuracy_m:5`, a phone can serve metres behind an OS permission prompt, IP geolocation is kilometres and should be named as such rather than degraded silently. Consumers must be able to refuse imprecise data rather than receive a plausible-looking wrong point.

### 3.4 `media.*` — typed artifacts first, raw streams only where realtime demands

The issue's "typed images/video vs raw byte streams". v0 recommendation: **typed artifacts by default, one raw channel by exception.**

- Typed: `media.image` / `media.audio` / `media.video` produce an artifact reference `{artifact_id, mime, bytes, sha256, width/height or duration_ms, created_at}`, stored like `records` and read back through an id. Bounded, deduplicable, replayable from the journal, and cheap for a text or pixel device to ignore. This is the shape a *world* wants: an artifact is data.
- Raw: `media.stream` is a subscription carrying frames, each at most `MAX_EVENT_BYTES` (64 KiB, the same ceiling as any event), with an explicit end and a backpressure signal. Only realtime preview and live voice need it. A raw stream is never the permanent record — it either terminates in an artifact or is discarded.

Refusal is where the difference lands: an artifact cap on a device with no storage is `unknown_cap` + `device_lacks_affordance`; a stream on a slow MQTT link is `outside_ceiling` + `rtt_class` in the reason.

### 3.5 `speech.*` — human conversation, two audio paths

For human-conversation mode (the issue's "client-side mic-to-text vs audio-to-server, possibly per-app"):

| Path | Where STT runs | Cost | Privacy | Quality | Fit |
| --- | --- | --- | --- | --- | --- |
| `speech.transcribe` | on the device/browser | bandwidth ~0, CPU/battery on device | audio never leaves the device | varies (browser engine, local model) | **default for v0** |
| `speech.capture` | server | upload per utterance + ASR price | audio to server, needs consent | consistent across devices | opt-in per app |
| `speech.synthesize` | either | negligible | text only | — | output side of the same mode |

Recommendation: **client-side transcription is the default**, because the world model only needs text and the existing `chat.say` / event path already carries it; `speech.capture` exists for devices that cannot run STT (most ESP32s) and for apps that need a better model. Per-app, not global: consent and provider choice differ per app, and the grant ledger's `actor_denied` is already the right mechanism for "this participant at this device said no".

### 3.6 Styling — semantic tokens, not CSS and not all-web-elements

The issue's third question: a styling tool across GUI and web that is neither all-web-elements nor Electron. The line this model takes is the one `world_presentation.rs` already draws: **style is declared semantically and resolved by the adapter.**

- v0 today: `ScreenCell` carries xterm-256 colour + bold — a lowest common denominator that means the same in a terminal, a browser and a 1-bit pixel screen.
- v0 proposed: a **token set** (`emphasis`, `muted`, `heading`, `code`, `warning`, `success`, `link`) attached as style runs on a surface, each adapter mapping tokens to its own CSS variables, ANSI attributes or dithering. Programs name intent; the adapter owns looks. This is neither "emit DOM" (breaks the trusted-adapter rule on ESP32 and terminals) nor "render in Electron" (drags a browser into a 128×64 display).

A full theme/design-system surface is explicitly out of scope (§9).

## 4. The seam: device affordances are not world grant policy

This is the one conceptual addition the taxonomy makes, and the reason it is not just more catalog names.

Today's grant model answers one question: **may this world program, in this world, on this node, have this server-side datum?** Owner sets policy, ceiling caps it, usage is counted, all local and unsigned.

Interface capabilities answer a different question: **may this program use this hardware, here, now, on behalf of the human holding it?** Three authorities meet, and they fail differently:

| | World caps (today) | Interface caps (this taxonomy) |
| --- | --- | --- |
| Granting party | world owner (server) | device (hardware + ceiling) and the human at it (consent) |
| Lifetime | standing grant, policy-managed | per-use for sensors; standing only for display |
| Where it is enforced | hosting node | the adapter on the device |
| Revocation | policy change, journaled | consent withdrawal, immediate, may arrive mid-stream |
| Failure mode | request refused, program continues | request refused *and* the program must have a degraded rendering (§8) |
| What it protects | the world's data | the person in front of the screen |

Consequences for the model, all additive:

1. **Two ledgers, one protocol.** The world ledger keeps granting data caps; the device keeps a local affordance/consent ledger. Both journal. Neither is signed. A program sees one `wants`/`result` stream and cannot tell which ledger answered — good, that is the point.
2. **Sensor caps are per-use by default.** A wildcard grant must never reach `loc.*`, `media.record`, `media.stream` or `speech.capture`; the existing `grant_on_use`/ceiling code already refuses to grant past a ceiling, and the ceiling for sensors is "not wildcard-grantable". This is a *refinement* of the existing ledger, not a second mechanism.
3. **Consent withdrawal is an event,** not just a refusal of the next request: `{"cap":"loc.watch","event":"revoked","reason":"consent_withdrawn"}` closes an open stream. Text-only and Screen programs are unaffected, which is why text-only mode matters beyond agents.
4. **Actor scope finally earns its keep.** `actor_denied` already exists for "the participant whose input caused the request lacks the scope"; for location and mic the actor *is* the human whose data it is. No new code concept, a much better fit.

## 5. ESP32 / MQTT: the three paths, compared

The issue names three: firmware speaking MQTT via Home Assistant, ID speaking MQTT directly, and direct web.

| | (a) Firmware → MQTT → Home Assistant | (b) ID speaks MQTT directly | (c) Direct web |
| --- | --- | --- | --- |
| Who owns the protocol | HA's entity model | ID | ID over WebSocket |
| Capability taxonomy applies? | no — caps terminate at HA | **yes**, same wants/result/refusal codes | yes |
| Surfaces | HA dashboard, not `WorldSurface` | pixel `screen` (small dims), `text` | full `Screen`, `Form`, media |
| Sensors | HA integrations (cheap, mature) | whatever the firmware exposes: GNSS, encoder, mic | browser APIs |
| OTA / lifecycle | ESPHome/HA gives it free | firmware owns it | n/a |
| Failure modes | two models, two grant authorities, HA is a de-facto ceiling | TLS + auth + reconnect + RAM in firmware | none new |
| Effort to first pixel | low | medium | low |

**Recommendation: (b) as the device path, (a) as interop, (c) unchanged.**

Rationale: the value of this taxonomy is that one program renders on a phone, a terminal and a 128×64 display with the *same* request/refusal semantics. Path (a) throws that away at the last hop and makes Home Assistant the authority for a capability model it does not have. Path (b) keeps one model and one refusal vocabulary; the firmware stays a thin adapter that implements only `device.info`, small `screen`/`text`, a closed `form.*` subset, `loc.position` when a GNSS module is present, and `key`/encoder events — which is exactly the part a constrained device *can* implement.

Concrete shape for (b), to be specified by a follow-up rather than here:

- Topic schema `id.device.v1`: `<prefix>/manifest` (published on connect, retained), `<prefix>/wants` (program's wants document, or a compact projection), `<prefix>/cap/result`, `<prefix>/surface`, `<prefix>/events`.
- One MQTT connection per device, per-world prefix, so a device may host several worlds without cross-talk.
- Frames reuse the event ceiling (64 KiB) and the args ceiling (8 KiB), so the wire bounds are the same numbers as on WebSocket. A 128×64 1-bit screen is ~1 KiB of cells — two orders of magnitude under the ceiling, which is why pixel screens fit the existing `Screen` variant with *smaller dimensions* rather than needing a new one.
- Rotary encoders are `Key` events extended with a signed delta in `args`, not a new input type.
- Power: `time.tick` already floors at 4 distinct intervals; the device path should add a minimum tick interval to avoid wake storms.

## 6. Per-platform notes

**Web.** Full `Screen` (canvas or DOM), `Form` native widgets, `loc.*` behind the browser permission prompt with accuracy from `coords.accuracy`, `media.record` via `getUserMedia`, `speech.transcribe` via the browser engine or a local WASM model. Ceiling: browser API availability, not hardware.

**ESP32 — pixel screen + rotary encoder.** `screen` with real dimensions (128×64, 64×32, 240×240), 1-bit or 8-colour, so `ScreenCell` colour must degrade (map xterm-256 to the nearest palette entry, dither tokens). Text entry is the hard part: no keyboard, so `form.*` on this device is choice-from-list and number-by-encoder only — which is why `form.*` must be a *closed field set with declared minimal requirements*, and why `text_entry:"none"` is in the manifest. Audio in only where a mic exists; audio out via I2S where wired. No camera on typical boards; GNSS via a module giving `accuracy_m` of a few metres, indoors `no_fix`.

**Phones.** The richest sensor device and the strictest about consent: OS-level permission prompts on top of the app's consent, both of which can be revoked outside our control, hence consent withdrawal as an event (§4.3). Location has accuracy tiers and staleness (`fix_age_ms`); battery is the real ceiling for `loc.watch` and `speech.transcribe`.

**Text terminals and agents.** The main mode (§7). No `form.*` beyond a prompt/answer projection, no sensors, no media beyond artifact ids printed as URLs or digests. A program must never *require* a surface to be useful here.

## 7. Text-only mode is the main mode

Not a fallback, for three reasons: it is what agents and scripts consume; it is the projection every other surface must degrade to; and it is what makes the same world testable in CI without a renderer.

The existing behaviour already half-supports it: `parse_surface` treats non-screen documents as legacy UTF-8 text, so a program that returns plain text is valid today. v0 recommendation: make the *contract* explicit rather than incidental.

- A **`text/1` presentation**: UTF-8 lines, no escapes, no cursor addressing, no redraw. Line-oriented append semantics — a script reading a pipe is the reference consumer.
- **Width-aware wrapping** from the manifest or the connection's declared columns, capped like `MAX_SCREEN_WIDTH`, defaulting to 80.
- **Every surface has a text projection**: `Screen` → lines (the existing `render_ansi` with colours stripped), `Form` → `field: value` lines plus numbered actions, `Media` → artifact id and digest, `Map` → coordinates. A surface with no text projection is not admissible.
- **No cursor redraw**: the mode exists precisely so that a log, a pipe or a transcript is readable, so a program that wants animation must ask for `Screen`.

## 8. WorldSurface extension sketch

Beyond today's single `Screen` variant, and ordered by what the taxonomy needs first:

| Priority | Variant | Why | Wire note |
| --- | --- | --- | --- |
| 1 | `Text(TextSurface)` | formalises the text-only contract in §7 | smallest possible document; a program may emit it without knowing the device |
| 2 | `Form(FormSurface)` | the input half of `form.*`; a `Screen` cannot express fields or actions | fields + actions + validation hints, bounded by `MAX_ARGS_BYTES` |
| 3 | `Media(MediaSurface)` | typed artifact references, the output half of `media.*` | artifact refs only — bytes never travel in a surface |
| 4 | `Map(MapSurface)` | location displays without inventing a drawing language | points + accuracy circles over a `Screen` background, not tiles |

What stays out of v0, deliberately:

- **A DOM/vector surface** — reintroduces program-supplied markup, breaks the trusted-adapter rule.
- **A styling system or theme document** — tokens (§3.6) only.
- **Inline video/audio playback** — artifacts and ids; playback is the adapter's business.
- **Multi-surface composition** (more than one surface at once) — one active surface per program keeps replay deterministic.
- **A layout engine or focus model** — `Form` has a field order and actions, nothing more.
- **`device.info` as a surface field** — it is a capability, so a program can ask when it cares and ignore it otherwise.
- **Camera/mic on ESP32**, phone-grade sensor fusion, and any PoC of the above: the issue's own rule forbids building the permanent feature on an unreviewed PoC.

Compatibility: all four are new `kind` tags on an existing tagged enum, so `parse_surface` gains arms and old documents keep parsing. No `WANTS_VERSION` bump; no catalog name changes meaning.

## 9. Open questions

1. **Who owns the device ledger's state across reconnects?** A device that drops and returns: does consent survive? v0 says sensor consent is per-use, so it does not need to; display affordances do, and they come from `device.info` on connect.
2. **Is `form.result` a capability result or a subscription event?** v0 says event, because the user may answer long after the request, possibly on another connection.
3. **Does `media.*` storage live in `records` or beside it?** Artifacts want dedupe and GC; `records.read` has neither. Follow-up needs a decision.
4. **One `device.info` per device or per program?** Per device, cached by the adapter, delivered as a result to whoever asks — otherwise every program pays for the manifest.
5. **MQTT QoS and retained `wants`:** retained wants makes reconnect cheap and risks replaying a stale request. Needs a decision in the bridge follow-up (recommend: retained manifest only, wants never retained).

## 10. Follow-ups

Filed for everything buildable, each linked from #2027 (links added on #2027 in the same pass as this document):

| Follow-up | Covers |
| --- | --- |
| `device.info` capability + manifest | §3.1, the root of adaptation |
| `text/1` presentation contract | §7, the main mode |
| `form.*` capability + `Form` surface | §3.2, §8 priority 2 |
| `loc.*` capability with precision ceiling | §3.3, §4 consent seam |
| `media.*` typed artifacts vs streams | §3.4 |
| MQTT device bridge `id.device.v1` | §5 recommendation (b) |
| `speech.*` client-side vs server transcription | §3.5 |

Not built here, and not built until reviewed: this record is design only; no code in `pkgs/id` changed.
