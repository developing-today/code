# World-VM Wasm portability audit — what of the worlds runtime runs inside a Worker

**Date:** 2026-10-30 · **Issue:** #2057 (wave-1) · **Epic:** #2056 · **Feeds synthesis:**
#2046 (ID serverless edition v1)
**Scope:** map `pkgs/id` world execution (wasmtime-hosted Roc/Wasm programs) against
Workers Wasm reality; classify features (isolate / native fallback / excluded); state the
hosted edition's promise list for app authors. Source: #2044 open question 1. Out of
scope: the serverless synthesis itself (#2046 consumes this) and the journal-equality
experiment (PoC #2271 owns it).
**Method:** this doc records the portability audit first produced by queue task 3359
(2026-10-29, on the issue) as the wave-pattern investigation document, re-verified
against `origin/main` (`3e3e7120`): host/guest split, fuel/memory/manifest machinery,
and example worlds confirmed in the tree; referenced issues stand as scoped (#2271 and
#2046 open). One drift corrected below (host version). This is an audit, not an
implementation and not a PoC.

---

## TL;DR verdict

The host doesn't port, and it doesn't need to: **the guest module is the portable
artifact; the host is replaceable.** The audit question is not "does the VM run in a
Worker" but: _for each thing the native host does for a guest, can an isolate host do
it, and which guests only use portable things?_ Two existing mechanisms do most of the
classification work: fuel/memory/payload caps (`world_limits.rs`) and the validated
world manifest (`world_manifest.rs`) — portability becomes one more validated manifest
declaration, checked at load time by each host tier. The contract below names exactly
what the hosted edition may promise app authors, and the hosted loader refuses the rest
at load with the reason named.

Workers limits used (per #2044): Wasm supported, SIMD yes, threading NO, WASI
experimental, 128 MB isolate memory, per-request CPU-time caps.

---

## 1. The framing

Today the host is **wasmtime** (native Rust; `pkgs/id/Cargo.toml`, `sandbox =
["dep:wasmtime", "world"]`) and the guests are Wasm modules (Roc programs compiled to
Wasm — e.g. `examples/tic-tac-toe/tic-tac-toe.wasm`). A Worker isolate cannot host
wasmtime — but it doesn't need to: the guest is unchanged; only the host is swapped for
an isolate-side implementation (`WebAssembly.instantiate` + DO-backed host calls).

Two mechanisms carry the classification:

- `src/world_limits.rs` — guests already run under **fuel, memory, and payload caps**,
  with per-world declared `wasm_memory_bytes` (validated, ≥ 64 KiB, clamped to the
  server ceiling). Fuel is a deterministic compute budget — the native analogue of the
  isolate's CPU-time cap.
- `src/world_manifest.rs` — worlds already carry a parsed, validated **manifest**.
  Portability becomes one more validated manifest declaration (see §3), checked at load
  time by each host tier — not a documentation promise.

---

## 2. Classification

| World-VM feature / surface                                                           | Isolate verdict                           | Notes                                                                                                                                                                                                                                                                                                                                                                     |
| ------------------------------------------------------------------------------------ | ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Guest Wasm module execution (single-threaded, fuel-bounded turns)                    | ✅ **Ports**                              | `WebAssembly.instantiate` in the isolate; the guest is unchanged. Turn-based invocation maps naturally onto request/response + DO state.                                                                                                                                                                                                                                  |
| SIMD-using guests                                                                    | ✅ Ports                                  | SIMD is supported in Workers Wasm.                                                                                                                                                                                                                                                                                                                                        |
| Journal / event-store host calls (append, read, fold)                                | ✅ Ports                                  | Reimplemented against DO SQLite (the #2044/#2045 per-user store). This is the core surface most world programs actually use.                                                                                                                                                                                                                                              |
| Clock / randomness as **journaled inputs**                                           | ✅ Ports, with discipline                 | Determinism rule (already implied by journaled worlds): time/randomness enter only as recorded host-supplied inputs, never ambient reads — under that rule an isolate host supplies them identically. A guest that reads ambient time breaks replay on _any_ host, so this is a program-correctness rule, not an isolate limitation.                                      |
| Key-value / records storage (`world_records`, `world_store`)                         | ✅ Ports                                  | DO SQLite / KV equivalents; payload caps from `world_limits` carry over.                                                                                                                                                                                                                                                                                                  |
| Presentation data (grids/screens as data — `world_presentation`, `world_line`)       | ✅ Ports as data                          | The _model_ is portable; renderers differ per surface (web on hosted, terminal/SSH native) — that was already the multi-surface design.                                                                                                                                                                                                                                   |
| Outbound HTTPS fetch (if a guest capability grants it)                               | ⚠️ Partial                                | Workers can fetch HTTPS; no raw sockets, no UDP, no QUIC/Iroh — `world_net` capabilities beyond HTTPS do not port.                                                                                                                                                                                                                                                        |
| Guests using Wasm **threads** / shared memory                                        | ❌ Native only                            | Threading is absent in Workers Wasm. (Roc world programs are typically single-threaded; this excludes a class, not the catalogue.)                                                                                                                                                                                                                                        |
| Guests whose declared `wasm_memory_bytes` > isolate budget                           | ❌ Native only                            | Mechanical check: isolate memory is 128 MB for the _whole isolate_ (runtime + all state), so the hosted ceiling for a guest declaration must sit well under it — recommended hosted cap: **64 MB** declared guest memory, leaving room for host + journal buffers. Manifests already carry the number; the hosted loader refuses above its ceiling with the reason named. |
| Long-running / unbounded compute (simulations that run minutes–hours per invocation) | ❌ Native only                            | Fuel maps to CPU caps only for _bounded_ turns. A guest whose single invocation can't finish inside the CPU cap doesn't port, however small its memory. Per-invocation bound is a manifest-declarable property (max fuel per turn).                                                                                                                                       |
| SSH surface (`world_ssh`), native terminal presentation                              | ❌ Native only                            | No SSH server exists in an isolate; hosted worlds are web-surface only.                                                                                                                                                                                                                                                                                                   |
| Filesystem access                                                                    | ❌ Native only (virtualized subset ports) | No fs in isolates; guests using only the records/store surface are unaffected (that's the point of the surface), guests with real fs capabilities stay native.                                                                                                                                                                                                            |
| **Compilation** of Roc sources (`world_compile`)                                     | ❌ Native only                            | The Roc compiler does not run in an isolate. Hosted edition loads **pre-compiled** modules only; authoring stays a native activity, distribution is the compiled module + manifest.                                                                                                                                                                                       |
| Multi-guest shared worlds with many simultaneous writers                             | ⚠️ Partial                                | A DO serializes per object (#2044) — a shared world hosted in one DO inherits that serialization: fine for turn-based/async worlds, a throughput ceiling for high-write-rate worlds. Classify per world by write pattern, not by hope.                                                                                                                                    |

---

## 3. The portability contract (what the hosted edition may promise app authors)

> **A world program is guaranteed to run in the hosted (isolate) edition, unmodified, if
> all of the following hold — each checkable from its manifest + module before it ever
> runs:**
>
> 1. Single-threaded Wasm (no threads/shared-memory proposals); SIMD allowed.
> 2. Declared guest memory ≤ **64 MB** (hosted ceiling; the native ceiling stays whatever
>    the node sets).
> 3. Bounded turns: every invocation completes within a declared per-turn fuel budget
>    sized to the isolate CPU cap — turn-based and request-shaped programs qualify;
>    continuous simulations do not.
> 4. Host calls limited to the **portable surface**: journal append/read, records/store
>    KV, clock/randomness _as journaled inputs_, presentation-as-data, and (optionally)
>    HTTPS fetch. No filesystem, no sockets/UDP, no SSH, no native-only capabilities.
> 5. Distributed as a **pre-compiled module + manifest**; the hosted edition never
>    compiles sources.
>
> Programs outside the contract are **native-node programs**: they keep full wasmtime
> hosting (threads, big memory, long compute, SSH, fs) and the hosted edition must
> refuse them at load with the failed condition named — a refused load with a reason is
> a kept promise; a program that starts in an isolate and dies on a limit is a broken
> one.

The manifest gains one validated field — `portability = wasm | native` (author's
declaration, host-verified against 1–4 where mechanically checkable) — so app authors
design against the contract at publish time, on either tier.

---

## 4. What does NOT change

Native hosting is untouched: wasmtime remains the full-capability host, and nothing in
the contract asks native programs to shrink. The hosted tier is a _subset with a
guarantee_, which is exactly #2044's conclusion about the whole serverless edition, now
made precise for worlds. Worlds whose state lives in the journal are portable in the
sense that matters most: the same journal folds to the same state under either host —
the property the follow-on PoC must demonstrate, since the whole contract rests on it.

---

## 5. Follow-on routing

- PoC follow-on **#2271** (one example world running **unmodified** in a Worker isolate
  with journal equality against the native host as the pass condition, plus the
  manifest-refusal case; enqueued as triage task 3426) was filed by task 3359 and is
  open — this write files nothing new. If journal equality fails, this audit's contract
  is wrong in a way only that experiment can show; that is what PoCs are for.
- This audit is an input to synthesis **#2046** (serverless edition v1), whose other
  inputs (#2044, #2045) are complete.
- **Not a PoC:** no code was built; the audit above is the deliverable.

## Verification note

Checked at `origin/main 3e3e7120`: issue #2057 open with no owning or duplicate PR;
`world_limits.rs` (fuel/memory/payload caps, `wasm_memory_bytes` validated ≥ 64 KiB)
and `world_manifest.rs` present in `pkgs/id/src/`; `examples/tic-tac-toe/` ships
`main.roc` + `tic-tac-toe.wasm` alongside `roc-world/`, `territory/`, `grid/`; #2271
and #2046 open as scoped. One drift from the 2026-10-29 audit text: the tree pins
`wasmtime = "=48.0.7"` (`pkgs/id/Cargo.toml:120`), not 43 — the host version moved;
the framing (host replaceable, guest portable) is unaffected. Prior audit: task 3359's
comment of 2026-10-29, recorded here as the wave-pattern doc with that one correction.
