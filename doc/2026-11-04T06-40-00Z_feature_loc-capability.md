# `loc.*` — location capabilities with an accuracy ceiling and per-use consent

Status: slice 1 (pure core, `crates/id-core/src/world_caps.rs`) implements the
catalog entries, the refusal vocabulary, the grant-ledger rules, the typed
reading, and the watch interval/revocation shapes for issue #2341, following
the capability taxonomy record
(`doc/2026-10-29T14-05-00Z_investigation_capability_taxonomy/`). Host wiring
in the world actor — executing the providers against a host location/consent
seam and closing open watches on withdrawal — is the follow-up slice. Until
it lands, no host serves a fix; nothing is fabricated.

## Capabilities

| Name           | Kind         | Arguments / spec                          |
| -------------- | ------------ | ----------------------------------------- |
| `loc.position` | request      | optional `max_accuracy_m` (whole metres)  |
| `loc.watch`    | subscription | `loc.watch` or `loc.watch:<milliseconds>` |

Both are **sensor caps**: they are excluded from default ceilings, a
wildcard (`*`) grant never reaches them, and grant-on-use never mints a
durable grant for them. A world reaches them only through an explicit
ceiling entry _and_ an explicit grant, and each use still passes consent.

## Readings

Every result and watch delivery carries the typed reading:

```json
{ "lat": 30.2672, "lon": -97.7431, "accuracy_m": 5.0, "fix_age_ms": 250, "source": "gnss" }
```

`source` is one of `gnss | network | ip | caller_supplied`. The source is
part of the reading: IP geolocation is kilometre-scale and is named `ip`,
never silently degraded to look like a GNSS fix.

## Refusals (existing codes, specific reasons)

| Code              | Reason                 | Meaning                                                                                                                                               |
| ----------------- | ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `outside_ceiling` | `precision_ceiling`    | the fix is less precise than the requested/allowed ceiling — consumers can refuse imprecise data instead of receiving a plausible-looking wrong point |
| `cap_denied`      | `consent_required`     | no per-use consent on record                                                                                                                          |
| `cap_denied`      | `os_permission_denied` | the OS refused the location permission                                                                                                                |
| `failed`          | `no_fix`               | no fix available (e.g. no sky view)                                                                                                                   |

Consent withdrawal is an event, not just the next refusal: an open
`loc.watch` receives
`{"cap":"loc.watch","event":"revoked","reason":"consent_withdrawn"}` and
the stream closes.

## Interval floor and battery

`loc.watch` has a **5-second interval floor** (`MIN_LOC_WATCH_INTERVAL_MS`).
GNSS fixes are power-hungry and take about that long to refresh; sampling
faster buys no accuracy. Expect a long watch to dominate a session's
battery budget on phones and ESP32 devices — programs that only need a
position should use one-shot `loc.position`. At most 4 concurrent watches
per program (`MAX_LOC_WATCHES`).

## Per-platform notes

- **Browser:** Geolocation API behind the browser permission prompt;
  `coords.accuracy` is the reading's `accuracy_m`; source is `gnss` or
  `network` as the browser reports the fix. Permission denial maps to
  `os_permission_denied`; no prompt shown yet maps to `consent_required`.
- **Phones:** richest sensor device, strictest consent — OS permission
  on top of app consent, both revocable outside our control (hence
  withdrawal as an event). Accuracy tiers (GNSS metres, network tens of
  metres) and staleness (`fix_age_ms`) are reported, not hidden. Battery
  is the real ceiling for long watches.
- **ESP32 GNSS:** a GNSS module serves `loc.position` at metre accuracy
  (`source: "gnss"`) when it has a sky view; indoors it answers `no_fix`
  rather than a stale or fabricated point. No network/IP fallback is
  silently substituted. `loc.watch` duty-cycles the module at the
  interval floor or slower.

## Out of scope

Geofencing, trail history, reverse geocoding, and any persistent location
store. Readings are delivered to the requesting program only.
