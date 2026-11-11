# `id.device.v1` MQTT device bridge — schema and PoC (#2343)

Status: **spec + labelled PoC**. Decided inputs from #5901 (closed):
retained manifest only, TLS plus per-device credentials, the closed device
subset, bridge-as-translator. Production hardware rollout stays deferred
(#2028); boards, firmware, OTA, ESPHome and power are #5783.

## Topic schema

One connection per device, one prefix per world. `PREFIX` is
`worlds/<world>/devices/<device-id>`:

| Topic | Direction | Retained | Document |
|---|---|---|---|
| `<PREFIX>/manifest` | device → bridge | **yes** (only) | §1 |
| `<PREFIX>/wants` | device → bridge | **never** | §2 |
| `<PREFIX>/cap/result` | bridge → device | no | §3 |
| `<PREFIX>/surface` | bridge → device | no | §4 |
| `<PREFIX>/events` | device → bridge | no | §5 |

A retained `wants` replays a stale request to the next subscriber, so the
bridge rejects retained wants publishes outright.

## Wire bounds

Identical to the host protocol (`id-core/src/world_caps.rs`):

* `wants` document: at most 64 KiB (`MAX_WANTS_BYTES`), 64 entries
  (`MAX_WANTS_ENTRIES`), names at most 64 bytes (`MAX_NAME_BYTES`).
* Request `args`: at most 8 KiB serialized (`MAX_ARGS_BYTES`); oversize
  args reject the whole `wants` document at parse time, the same boundary
  as the host path.
* Events and the manifest: at most 64 KiB (`MAX_EVENT_BYTES`).

## 1. Manifest (retained, on connect)

`DeviceManifest::to_json` (`world_caps.rs`): `manifest.v = 1` plus the five
sections. Example — pixel screen, keys plus encoder, GNSS, no mic/camera:

```json
{
  "manifest": {"v": 1},
  "display": {"kind": "pixel", "width": 128, "height": 64, "colors": 1},
  "input": {"keys": 8, "encoder": true, "pointer": false, "text_entry": "none"},
  "sensors": {"gnss": {"accuracy_m": 5}, "microphone": false, "camera": false},
  "audio": {"out": "pcm", "in": null},
  "network": {"transport": "mqtt", "rtt_class": "slow"}
}
```

A 128×64 1-bit screen is ~1 KiB of cells, which is why pixel screens are
the existing `Screen` variant at smaller dimensions rather than a new kind.

## 2. Wants (never retained)

The host `wants` document (`parse_wants`): optional `v = 1`, `subscribe`
array, `requests` array of `{id, cap, args}`. Example:

```json
{
  "v": 1,
  "subscribe": ["time.tick:1000", "players"],
  "requests": [
    {"id": "r1", "cap": "device.info", "args": {}},
    {"id": "r2", "cap": "loc.position", "args": {"fix": "cached"}}
  ]
}
```

## 3. Cap result (one verdict per request, document order)

```json
{"id": "r1", "cap": "device.info", "ok": true}
{"id": "r9", "cap": "bogus.cap", "ok": false, "code": "unknown_cap", "reason": "capability bogus.cap is not in the catalog"}
{"id": "r7", "cap": "device.info", "ok": false, "code": "cap_denied", "reason": "no grant for device.info"}
```

Refusal codes reuse the web vocabulary: `unknown_cap`, `cap_denied`,
`failed` (oversize args reject the document at parse, before verdicts).
Location refusals reuse the #2341 codes
(`cap_denied` with `consent_required` / `os_permission_denied`,
`outside_ceiling` with `precision_ceiling`, `failed` with `no_fix`).

## 4. Surface

The current surface document for the device's screen subset, e.g.:

```json
{"v": 1, "screen": {"kind": "pixel", "width": 128, "height": 64, "cells": "<base64 1 KiB>"}, "text": {"lines": ["id ready", "world abc"]}}
```

## 5. Events

Device → bridge input events. A rotary encoder reports as a `Key` event
with a signed `delta` in `args`:

```json
{"protocol": "id.device.v1", "kind": "Key", "key": "volume", "args": {"delta": -3}}
```

## Recommendations (open decisions, recorded)

* **Retained:** manifest only. Wants are never retained.
* **Auth:** TLS plus per-device credentials, provisioned out of band;
  fail closed (unknown device or wrong token refuses before any
  publish/delivery). No oracle: the refusal says nothing about which half
  was wrong.
* **Reconnect/backoff:** exponential from 500 ms, capped at 30 s
  (`backoff_delay_ms`). The manifest republishes on every connect, so a
  reconnect needs no other recovery step.
* **Device-side journaling:** the device journals locally (ticks, events)
  across reconnects; the journal is the miniature form in `FakeDevice`.
* **Minimum `time.tick`:** 1 Hz (`MIN_TIME_TICK_MS = 1000`) for the device
  path — slower than the `loc.watch` 5 s floor allows for watches, and
  chosen to avoid wake storms on ESP32-class radios.
* **Firmware subset (closed):** `device.info`, small `screen`/`text`, a
  closed `form.*` subset, `loc.position` when GNSS is present, key/encoder
  events. Nothing else without a new decision.

## PoC

`pkgs/id/src/device_bridge.rs` (labelled PoC in its module docs): topic
helpers, fail-closed `Authenticator`, `Bridge::evaluate_wants` with the
web refusal vocabulary, `FakeBroker` (retained manifest, retained-wants
ban, bounds, auth), `FakeDevice` (journal, minimum tick, backoff,
encoder-as-Key), and integration tests proving manifest/wants/result,
refusal, stale retained-wants rejection, bounds enforcement and
fail-closed auth. A real MQTT transport replaces `FakeBroker` without
touching `Bridge`. Home Assistant interop is recorded as
out-of-scope-but-supported: HA may bridge to the same topics, but the ID
capability model and refusal vocabulary stay authoritative.
