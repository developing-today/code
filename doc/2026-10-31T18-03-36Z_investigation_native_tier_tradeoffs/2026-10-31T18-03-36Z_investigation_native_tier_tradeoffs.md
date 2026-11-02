# Investigation: the native tier vs WASM — fuel, trust, and when each is right

- **Date:** 2026-10-31
- **Question (Drew, research only — no change requested):** Roc programs can
  only do what their platform allows. If the platform side is guaranteed
  safe, is the WASM sandbox still necessary — losing deterministic fuel and
  falling back to per-call wall-clock timeouts? Would an accepted class of
  worlds (our own Explorer, chats) be faster running a trusted platform
  directly? What does wasmtime actually cost?
- **Method:** the current pkgs/id source (every load-bearing claim names
  its file) and the fuel baselines recorded in `src/sandbox.rs`.
  Performance figures in §4 are labelled estimates, not measurements.

## Verdict up front

**The tier Drew describes already exists** (`src/world_native.rs`), and it
implements exactly his trade: OS hardening in place of the format
guarantee, a wall-clock deadline in place of fuel, admin trust in place of
peer distribution. WASM remains the right default — not for raw safety,
but for the two things only it provides: **deterministic metering** and a
**distributable guarantee**. Running first-party programs native is a
policy question (which programs the standard install ships that way), not
new mechanism. At today's call scales, wasmtime's cost is small enough
that speed alone does not justify moving anything (§4).

## 1. What the native tier is (facts, `src/world_native.rs`)

- A Roc app built for the platform's `x64musl` target is a statically
  linked worker binary (`examples/roc-world/host/src/native_worker.zig`)
  serving the same program ABI over stdin/stdout frames. The host spawns
  it and drives it exactly like a Wasm guest.
- Isolation is the OS: `no_new_privs`; rlimits (address space, CPU, file
  size, processes, core); a seccomp allowlist — no `open*`, no sockets,
  no `clone`/`execve`/`ptrace`, and `mmap`/`mprotect` may not make memory
  executable. An unknown syscall kills the process.
- Default limits (`NativeLimits`): 256MB address space, 10s CPU, and a
  **10s wall-clock deadline per call**. A crash, timeout, protocol
  violation, or oversize reply poisons the program, exactly like a Wasm
  trap.
- Trust posture, in the module's own words: "Unlike Wasm, the guarantee
  is the OS sandbox, not an import-free format: native modules are
  admin-installed and opt-in (`--world-native`), and are never offered
  for peer download."

## 2. The metering trade: determinism (conclusion)

Fuel is deterministic — the same call burns the same fuel on every
machine, forever. A wall-clock deadline is not: a call that takes 9s on a
loaded laptop and 2s on an idle server poisons the program on one host
and not the other. The native tier's 10s default — for calls that
normally take milliseconds, with the comment "correctness does not
depend on snappy guests" — is the mitigation: make the deadline so
generous the non-determinism almost never bites.

Because execution is host-local and the journal syncs _outcomes_ (the
signed event layer carries trust and ordering — see the bridge
investigation, `doc/2026-10-29T11-27-00Z_investigation_bridge_semantics_hosted_native`),
the divergence shape is an availability wrinkle — one host's copy of a
program dies under load where another's wouldn't — not a consensus fork.
That is an acceptable risk for programs an operator deliberately trusts.
For peer-supplied programs, where admission must be trustless and
identical everywhere, it is not.

## 3. The distribution trade (conclusion)

A WASM module is safe to offer to strangers: any node can verify the
format-level guarantee from the artifact itself and run it with
identical behaviour and identical metering. A native binary asks each
node's admin to trust it. This — not performance, not even safety in the
small — is why WASM is the default and the only peer-distributable
format, and why native is opt-in per install.

## 4. What wasmtime costs (estimates)

For this system's call profile (baselines from ~700 to ~390,000 fuel per
call, `src/sandbox.rs`):

- **Compute:** Cranelift-compiled WASM typically runs at ~70-95% of
  native speed on compute-bound code — a ~5-40% tax, workload-dependent.
  In absolute terms it is small here: wasmtime burns on the order of
  10^8-10^9 fuel/second, so even the heaviest layout call (~390k fuel)
  is around a millisecond of execution; the tax is a fraction of that.
- **Fixed per-call costs** dominate at this scale: marshalling (copying
  inputs in and replies out — the call-kind budgets in `src/sandbox.rs`
  explicitly include "the input allocation and free around it") and
  host-call transitions for capability requests.
- **Native is not free either:** an IPC frame round-trip per call, a
  resident process per program, spawn + platform init at start. For
  chatty small calls it can be a wash; it wins on sustained single-call
  compute and on skipping Cranelift compilation at install (which the
  module cache mostly amortizes anyway).

**Estimate:** Explorer- and chat-scale programs would not be perceptibly
faster on the native tier — perhaps 10-30% less CPU on heavy renders,
nothing user-visible. The honest reasons to choose native for a
first-party program are platform reach (capabilities awkward in WASM)
and debuggability (real processes, real profilers), not speed.

## 5. The "accepted platforms" variant (conclusion)

The shipped tier trusts at the level of the _program binary_ (an admin
installs this executable). Trusting a list of _platforms_ instead —
any program built on an accepted platform runs native — widens the
blast radius in one specific way: Drew's purity argument still holds (a
program cannot exceed its platform's capability surface, and ceilings
plus deadlines still bound it), but a platform bug is now exploited with
native consequences rather than a Wasm trap, and the unit of review
becomes the platform + toolchain rather than each program. A second, quieter gap: fuel doubles as _accounting_ (the `world measure` report,
per-call-kind usage). Deadlines provide almost no accounting; if native
use grows, the measure/quota story needs a replacement (getrusage per
call is the obvious candidate, still machine-dependent as a _limit_,
usable as a _meter_).

## 6. Open questions

1. Is any first-party program actually guest-compute-bound today? The
   recorded baselines say no; revisit if a program's baselines grow by
   an order of magnitude.
2. Should the standard install ship Explorer/chat as native workers by
   default (policy), accepting §2's residual non-determinism for
   programs every node operator already trusts by shipping the host?
3. If the native share grows, what replaces fuel as the usage meter
   for quotas and the measure report?

## Sources

- `pkgs/id/src/world_native.rs` — the native tier (limits, sandbox,
  deadline, trust posture).
- `pkgs/id/src/sandbox.rs` — GuestCall kinds, fuel budgets and baselines.
- `pkgs/id/src/world_caps.rs` — the capability catalog and ceilings.
- `doc/2026-10-29T11-27-00Z_investigation_bridge_semantics_hosted_native/`
  — why execution locality keeps the journal semantics unchanged.
- `doc/2026-10-31T17-47-20Z_design_world_presentation_and_fuel/` — the
  presentation model both tiers serve.
