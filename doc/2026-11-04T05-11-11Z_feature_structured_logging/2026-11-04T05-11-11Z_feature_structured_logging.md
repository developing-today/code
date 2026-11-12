# Structured operational logging for `id` — design record

Issue: #1791 (split from rollup #1294, triage task 2544). Implementation:
queue task 9175, PR #5287. Record created 2026-11-04T05:11:11Z during
rollout, per `docs/DOCUMENTATION_PROTOCOL.md`; the implementation and
this record land in the same PR.

## Request / intent

`id` initialized tracing as plain text to stderr with level resolution
only (`--log-level` / `RUST_LOG` / `LOG_LEVEL` / `DEBUG`, #2322). The
remaining #1791 scope asked for app-wide _operational_ log hygiene —
explicitly distinct from the F12.6 world audit/telemetry records of
#1063:

- JSON log lines with trace ids and context properties on server logs.
- Nanosecond RFC 3339 timestamps with a timezone: host tz default,
  UTC fallback, explicit override.
- Log-to-file with a configurable location and per-destination levels
  (e.g. console at info while a file takes debug).
- Plain behavior unchanged by default, or the change documented.

## Design

### Configuration contract (environment variables)

Configuration is environment-only, extending the existing env-driven
level resolution rather than adding CLI flags (rationale below):

| Variable            | Values                                                     | Default                       |
| ------------------- | ---------------------------------------------------------- | ----------------------------- |
| `ID_LOG_FORMAT`     | `text` \| `json`                                           | `text`                        |
| `ID_LOG_FILE`       | filesystem path                                            | unset (no file destination)   |
| `ID_LOG_FILE_LEVEL` | any `EnvFilter` string, e.g. `debug`, `id=trace,iroh=info` | the console's resolved filter |
| `ID_LOG_TZ`         | IANA name, `UTC`, `local`, fixed offset `+05:30` / `-0400` | `local`                       |

Resolution lives in the library as pure functions
(`id::structured_logging::LogConfig::resolve`, `resolve_zone`) that
take raw values, so the whole table is unit-testable without mutating
process environment; `main.rs` reads the environment and calls them,
mirroring the existing `resolve_log_level` split.

### Layers and destinations

`init_logging` in `main.rs` builds a `tracing_subscriber` registry:

- A console layer on stderr, always present, filtered by the existing
  resolved filter string (base level + noisy-module caps, #2322
  behavior untouched).
- A file layer, present only when `ID_LOG_FILE` is set, with its own
  `EnvFilter` (the `ID_LOG_FILE_LEVEL` override or the console filter)
  — this is what makes the two destinations' levels independent.
- Both layers share the configured format and timestamp timer.
- Layers are collected as a `Vec` of boxes on the registry: stacking
  two individually boxed layers does not type-check, because a layer
  boxed for `Registry` is not a `Layer` for the already-layered stack.

The file is opened append-mode with parent directories created
(`structured_logging::SharedFileWriter`, one file behind a mutex,
cloned per event by the writer factory).

### JSON shape and trace context

The JSON format is `tracing_subscriber`'s JSON formatter (the `json`
feature; `Cargo.lock` gains only `tracing-serde`). Each line carries
`timestamp`, `level`, `target`, and the event's fields; with
`with_current_span(true)` and `with_span_list(true)` it also carries
the current span's name and fields and the span list — the trace
context available in-process. Wiring OpenTelemetry-style trace-id
propagation across services is deliberately **not** in this scope:
the packet bounds it out ("smallest necessary trace/span context
wiring"; broad cross-service protocol changes would be a follow-up),
and #1063 owns audit/telemetry export design.

### Timestamps

A custom `FormatTime` (`Rfc3339TzTimer`) renders `SystemTime::now()`
as RFC 3339 with `SecondsFormat::Nanos` in the resolved zone, using
`chrono` (already a dependency) and `chrono-tz` (already a dependency)
for DST-aware named zones. Fixed offsets and named zones are parsed
by `resolve_zone`; an unrecognized value resolves to UTC and sets a
fallback flag.

### Failure behavior

Logging must never kill the node it observes:

- An unopenable `ID_LOG_FILE` warns on stderr (before tracing starts)
  and logging continues console-only.
- An unrecognized `ID_LOG_TZ` warns the same way and falls back to
  UTC.

### Deliberate default change

Timestamps are rendered in the host-local zone with nanosecond
precision by default, replacing the fmt layer's previous UTC default.
This is the #1791 ask ("host tz default"), and it is the _only_
default-output change: format, destination, levels, and filter
resolution are unchanged when no `ID_LOG_*` variable is set.

## Alternatives considered

- **CLI flags for the new knobs.** Rejected for this slice: level
  configuration is already split between two flags and three env
  vars; adding four more flags widens the CLI surface and the
  parse-order interactions (logging initializes before most of the
  program runs). Env-only keeps one documented contract and matches
  how the packet scoped it ("initialization/configuration building on
  the existing setup"). Flags can be layered on later without
  changing the contract, since resolution is a pure function of raw
  values.
- **`tracing-appender` for the file destination.** Rejected: a new
  dependency (plus its non-blocking worker/rotation machinery) for
  what is one append-mode file; `SharedFileWriter` is ~30 lines and
  the rotation question is an operator concern at this scale.
- **Custom JSON serialization.** Rejected: the subscriber's JSON
  formatter already emits the required shape; hand-rolling invites
  drift.

## Verification

- Library unit tests (`cargo test --lib structured_logging`, 10):
  config defaults and parsing, exact RFC 3339-nanos rendering for
  UTC / named zone (DST-aware) / fixed offset, bad-zone fallback,
  independent destination filters, shared-writer appends, a JSON
  probe asserting timestamp shape + fields + span context, and the
  ANSI-isolation test described in the update below.
- CLI integration tests (`tests/cli_integration.rs`,
  `structured_logging_tests`): the real binary with the env vars set —
  JSON file destination with an independent debug level while the
  console stays at warn, text file output free of ANSI escapes, and
  both startup fallbacks (bad tz, unopenable file) warning while the
  command still succeeds.
- Existing binary tests (`cargo test --bin id`, the #2322 log-level
  table) unchanged and passing.

---

## 2026-11-04T05-11-11Z Update: Codex review findings on PR #5287

Three findings from the Codex review of the initial implementation,
all accepted and fixed in the same PR:

1. **This design record** was missing (pkgs/id AGENTS.md requires a
   dated record for significant features). Created — this file.
2. **CLI integration coverage** was missing (AGENTS.md: features get
   `tests/cli_integration.rs` cases). The library tests resolved
   configuration but never exercised the production env-reading path
   or `init_logging`. Added the `structured_logging_tests` module
   described above.
3. **ANSI escape leak into the file destination (P2).**
   `tracing_subscriber` caches each span's formatted fields in the
   span's extensions, keyed by the field formatter's _type_. Both
   layers used `DefaultFields`, so the console layer — formatting
   first, with ANSI enabled — populated the shared cache with styled
   fields (`marker` rendered with italic/dim escapes) and the file
   layer reused them despite `.with_ansi(false)`. Fix:
   `structured_logging::PlainFields`, a newtype wrapping
   `DefaultFields` used only by the file layer, gets its own cache
   slot formatted without ANSI; rendered text is otherwise identical.
   The regression test
   (`structured_logging_file_layer_span_fields_stay_plain`) was
   verified to fail without the fix (file output contained
   `\u{1b}[3m` sequences) and pass with it.
