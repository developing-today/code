# Authority-boundary audit — Phase 12 exit criterion 9

Audited 2026-11-23T02:02Z against `main` at `9794191ad`, for
[#12597](https://github.com/developing-today/code/issues/12597) and the
Phase 12 exit audit
[#12231](https://github.com/developing-today/code/issues/12231), row 9.

Criterion 9: "No new file, network, or process access except through a
named, ceilinged provider."

## Verdict

**No bypass found.** Every guest-reachable surface that can touch files,
network, or processes resolves to a named provider in the registry, and
every name outside the registry fails closed before any effect. Three
non-bypass findings (F-1, F-2, F-3) are recorded below; none re-opens
row 9. The regression test pinning the dispatch boundary is
`authority_boundary_tests` in
`pkgs/id/crates/id-worlds/src/world/actor.rs`.

## Method

The audit enumerates the guest's execution environment first, then
every channel by which guest code can cause a host effect, then every
remaining file/network/process call site in the two crates that own the
boundary (`id-caps`, `id-worlds`) and classifies it guest-reachable or
host/operator-only. "Guest-reachable" means reachable from world
program code with no operator action and no admin token. A surface
passes when (a) it names a registry provider, (b) the ledger
grant/ceiling decision precedes the effect, and (c) unknown names are
refused before any effect.

## The registry (the named set)

`pkgs/id/crates/id-caps/src/world_caps.rs`, `define_provider_registry!`
(line 629), assembled into `CATALOG_ENTRIES`: **35 entries** — 34
production host entries plus 1 client-side entry.

| Kind             | Count | Names                                                                                                                                                                                                                                                                                                                                        |
| ---------------- | ----- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Request (host)   | 23    | `time.now`, `random.u64`, `players.list`, `chat.say`, `chat.history`, `records.read`, `kv.get`, `kv.set`, `kv.delete`, `blob.put`, `blob.get`, `blob.delete`, `http.fetch`, `notify`, `schedule.at`, `schedule.cancel`, `world.info`, `device.info`, `form.present`, `form.cancel`, `loc.position`, `speech.transcribe`, `speech.synthesize` |
| Subscription     | 4     | `time.tick`, `players`, `loc.watch`, `speech.capture`                                                                                                                                                                                                                                                                                        |
| Event            | 1     | `form.result`                                                                                                                                                                                                                                                                                                                                |
| PlatformModule   | 6     | `Key`, `Screen`, `RocJson`, `RocGrid`, `RocCap`, `RocText`                                                                                                                                                                                                                                                                                   |
| Request (client) | 1     | `client.file.read`                                                                                                                                                                                                                                                                                                                           |

Every entry carries a version, a kind, an argument schema, bounds, a
required scope, and a default-ceiling membership. The outward-facing
providers (`http.fetch`, `notify`, `loc.*`, `speech.*`,
`client.file.read`) are `Excluded` from default ceilings. `is_known`
(world_caps.rs:1428) is the single name test; `CapLedger::decide_for`
(world_caps.rs:2330) denies unknown names with `unknown_cap` before
any ceiling, scope, or grant check.

## The enforcement chain (one channel, three gates)

Guest code has no direct host-call ABI. The Roc platform
(`pkgs/id/examples/roc-world/platform.roc`) exposes only pure modules
and host-called entry points (`init`/`update`/`view`/`layout`/
`records`/`wants`/`snapshot`/`restore`); `RocCap.roc` is pure Roc
that builds `wants` JSON documents. The guest's only outbound channel
is the `wants` string, and it passes three gates in order:

1. **Parse** — `parse_wants` (world_caps.rs:1536) bounds the document
   (size, entry counts, arg bytes, duplicate ids) but tolerates
   unknown cap names. Toleration is safe because gate 2 denies them.
2. **Decide** — `CapLedger::decide_for` denies unknown names
   (`unknown_cap`), then enforces ceiling, actor scope, and grant, in
   that order. `set_allowed` drops names outside the catalog.
3. **Dispatch** — `WorldActor::execute`
   (`pkgs/id/crates/id-worlds/src/world/actor/execution.rs:1198`)
   refuses client-side providers outright
   (`client_provider_async`, never executed on the host), refuses
   `form.result` as event-only, then matches the cap name to exactly
   one executor, with a fail-closed default arm
   (`_ => Err(CapError::unknown())`). Client-side requests go to the
   provider broker (`world/provider_broker.rs:290`), which refuses
   non-client manifests, ungranted, unattributed, and unoffered
   requests before any frame is sent.

Subscriptions go through `apply_subscriptions`/`arm`
(execution.rs:952, 1029): a denied or unknown subscription is refused
as an event and never armed. The compile gate `check_imports`
(world_compile.rs:149) refuses imports of platform modules the world
has not granted; grants can only name registry names (gate 2), so an
unregistered module can never be granted.

## Surface inventory

### Guest execution tiers

| Surface                         | Verdict                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Wasm tier (`world/sandbox.rs`)  | **Pass.** Import-free validation: `imports.is_empty()` is enforced at compile (sandbox.rs:166); "no WASI, filesystem, sockets, clocks". The host calls `plaza_*` exports only.                                                                                                                                                                                                                                                                                                                            |
| Native tier (`world_native.rs`) | **Pass on Linux; refused elsewhere.** ELF worker subprocess with `no_new_privs`, rlimits, and a seccomp allowlist (no `open*`, no sockets, no `clone`/`execve`/`ptrace`, no executable `mmap`); unknown syscall kills the process. `harden` bails "the native tier requires Linux" off Linux. Only channel is stdin/stdout frames. Native modules are admin-installed and server opt-in (`--world-native`); install requires the admin token (`world_session/service.rs:290`, `authorize_admin` at :488). |

### Host request executors (all 23 Request-kind host entries)

| Provider                                               | Verdict                                                                                                                                                                                            |
| ------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `time.now`, `random.u64`, `players.list`, `world.info` | **Pass.** Pure host reads; no files, network, or processes.                                                                                                                                        |
| `kv.get/set/delete`                                    | **Pass.** The world's own in-memory map plus its journal; keys validated (`validate_kv_key`); no guest-controlled path.                                                                            |
| `blob.put/get/delete`                                  | **Pass.** Content-addressed: the host computes the hash from the bytes (`execute_blob_put`, execution.rs:1806); stored in the world's journal. No guest-controlled path.                           |
| `http.fetch`                                           | **Pass.** Admin allowlist, re-checked on every redirect hop (`world/http_fetch.rs:72`); `host_not_allowed` off-list. The only guest-reachable outbound network path, and it is the named provider. |
| `notify`                                               | **Pass.** Rate-limited, journaled as a `Notification` event; target fixed to the world's own users.                                                                                                |
| `records.read`                                         | **Pass.** Byte-bounded, prefix-scoped page of the world's own published records.                                                                                                                   |
| `schedule.at/cancel`                                   | **Pass.** Host timers only.                                                                                                                                                                        |
| `chat.say/history`                                     | **Pass.** Host event log only.                                                                                                                                                                     |
| `form.present/cancel`, `form.result` (event)           | **Pass.** Forms validated against the schema and size bounds; the result channel is authorized separately (`authorize_form_result`).                                                               |
| `device.info`                                          | **Pass.** Reads the adapter boundary's cached manifest; never enumerates the device; `unknown_cap` when the host cannot describe itself.                                                           |
| `loc.position`                                         | **Pass.** Consent-gated (`consent_required` without consent); with no fix, `no_fix` — a reading is never fabricated.                                                                               |
| `speech.transcribe`, `speech.synthesize`               | **Fail closed, no executor — finding F-1.**                                                                                                                                                        |

### Subscriptions (all 4)

| Subscription                        | Verdict                                                                                                                                                                                          |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `time.tick`, `players`, `loc.watch` | **Pass.** Armed only when granted and inside the ceiling; tick floor and loc-watch consent/interval floors enforced at arm and fire time; no host fix means no delivery.                         |
| `speech.capture`                    | **Fail closed, no delivery — finding F-1.** Parses to `Subscription::Unknown`, is decided like any name, but `arm` has no branch for it: a granted subscription arms nothing and never delivers. |

### Platform modules (all 6)

`Key`, `Screen`, `RocJson`, `RocGrid`, `RocCap`, `RocText` are pure
Roc files in `pkgs/id/examples/roc-world/`; the platform `exposes`
exactly these six. None is a host function. The compile gate refuses
an ungranted import. See F-2 for the one name where the gate's list
and the registry disagree.

## Host/operator-only call sites (not guest-reachable)

Every other file/network/process call site in `id-caps` and
`id-worlds` was classified. None is reachable from guest code:

- **Journal/store persistence** (`world_store.rs`, `world_storage.rs`,
  `world_records.rs`, `world/replica.rs`): the host's own store; paths
  derive from the world's store directory, never from guest input.
  Guest data reaches it only through the `kv`/`blob` providers and
  journal replay of host-written entries.
- **Compile** (`world_compile.rs`, `roc` subprocess): admin-token
  gated (`compile_and_install`), native requires the server opt-in,
  output validated import-free (Wasm) or ELF (native) before install.
  Guest code cannot trigger a compile.
- **Module install/download** (`world_session/service.rs`,
  `world_session/mod.rs`): install is admin-gated; download serves
  the active module to a join-capability holder, or any stored module
  to an admin. See F-3.
- **Sync/replication** (`world_net.rs`, `world_replication_net.rs`,
  `world/replication_receiver.rs`, `program_registry_net.rs`,
  `session_host.rs`): iroh transport moves session frames; the join
  capability inside the session is the only authority. Guest programs
  do not speak iroh.
- **SSH presentation** (`world_ssh.rs`): an SSH client plays a world;
  keystrokes become `input` frames. The SSH login grants nothing; the
  capability or admin secret inside the frames is the only authority.
  Command mode offers only world-scoped reads (`/records`,
  `/documents`). No shell is spawned for a guest.
- **Terminal rendering** (`world_terminal.rs`): presentation only
  (probe/demux/render). No process spawn, no PTY.
- **CLI REPL** (`id-cli/src/repl/`): `sh -c` exists for the operator's
  own REPL. It is the operator's shell, not a guest surface.
- **Dev/doctor tools** (`world_dev.rs`, `world_doctor.rs`):
  operator CLIs; `world dev` runs a disposable scratch world isolated
  from every named world.
- **Directory mailer** (`id-directory/src/directory_mail.rs`): runs
  an operator-configured program. Not guest-reachable.
- **Test-only hits** (`world/actor.rs`, `sandbox.rs`,
  `world_native.rs` test modules): `std::fs` uses inside
  `#[cfg(test)]` fakes and tests, including the seccomp outlaw test.

## Findings (no bypass)

### F-1 — Speech providers are registered but unexecuted

`speech.transcribe`, `speech.capture`, and `speech.synthesize` are in
the registry with `Excluded` default ceilings, but no executor, arming,
or delivery path exists anywhere in `id-worlds`, `id-caps`,
`id-native-shell`, or `id-cli` (the only other "speech" mentions are
the gallery's "operator's speech" prose). A granted
`speech.transcribe`/`speech.synthesize` request fails closed with
`unknown_cap` (`execute` default arm); a granted `speech.capture`
subscription arms nothing and never delivers. This is dead
registration, not a bypass: no guest effect is possible through these
names. It is pinned by `authority_boundary_tests` so the gap fails
loudly instead of silently, and it should be resolved by either adding
the executors or removing the registrations — but that is follow-up
work, not a criterion-9 blocker.

### F-2 — The compile gate's module list includes a non-module file

`platform_modules` (world_compile.rs:122) lists every `.roc` file in
the platform directory except `platform.roc` — including
`platform-no-caps.roc`, which is an alternate platform definition,
not a module, and has no registry entry. The gate therefore lists 7
modules where the registry has 6 platform modules. Fail-closed: the
name can never be granted (the ledger filters grants to registry
names), so an import of it is always refused. The discrepancy should
be resolved by excluding `platform-no-caps.roc` from the gate list or
registering it, but no guest effect follows from it today.

### F-3 — Native modules are downloadable despite the "never offered" doc claim

`world_native.rs` documents native modules as "never offered for peer
download", but `download_module_session`
(`world_session/mod.rs:752`) serves the active module to any
join-capability holder with no native exclusion, and
`download_module_admin_session` serves any stored module to an admin.
Downloading is not installing — install still needs the admin token
and a native-enabled server — and a guest program cannot invoke a
module download at all (it is a session-protocol frame, not a `wants`
request). So this is a doc/code mismatch on a participant surface,
not a guest bypass. The doc claim or the serving path should be
corrected; no guest effect follows from it today.

## Regression test

`authority_boundary_tests` in
`pkgs/id/crates/id-worlds/src/world/actor.rs` pins the dispatch
boundary at the `execute` level, with every registered provider
granted and inside the ceiling:

- every host-side Request-kind registry entry dispatches to an
  executor (not `unknown_cap`), except the two F-1 speech requests,
  which are asserted to fail closed with `unknown_cap`;
- unregistered names (`not.a.provider`, `kv`, `http`,
  `client.file.write`, `speech`, `form.unknown`) fail closed with
  `unknown_cap`;
- `client.file.read` never executes on the host
  (`client_provider_async`);
- `form.result` is refused as event-only, and subscription names
  named as requests fail closed.

Run: `cargo test --manifest-path pkgs/id/Cargo.toml -p id-worlds --lib authority_boundary`

## Evidence limits

- This audit is a code audit of `main` at `9794191ad`, not a live
  penetration test: no malicious guest program was run against a live
  host during this audit. The Wasm import-free validation and the
  native seccomp allowlist are enforced by the code paths cited, and
  each has in-tree tests (sandbox import refusal; the seccomp outlaw
  test in `world_native.rs`), but this audit did not re-run those
  suites on the reference laptop.
- The regression test in this slice was run locally in the sandbox:
  `cargo test --manifest-path pkgs/id/Cargo.toml -p id-worlds --lib
authority_boundary` — **4 passed, 0 failed** in 0.10s (lib compile
  ~14m33s). A reference-laptop re-run was not available in this run
  (the fleet shell leg was down: tailnet seat logged out,
  control-plane 400 on re-register).
- The audit covers `id-caps` and `id-worlds` plus the guest ABI
  (`examples/roc-world/` platform). It does not re-audit the
  host/operator surfaces (CLI, web UI, directory) beyond classifying
  them as not guest-reachable; their own authorization (admin tokens,
  join capabilities) is asserted from the cited code, not re-proven.
