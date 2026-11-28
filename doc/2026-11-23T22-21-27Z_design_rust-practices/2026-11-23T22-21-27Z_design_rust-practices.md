# Rust practices & opinions (seed record): Frugal Async Rust considerations

> **STATUS: ALPHA / WORK IN PROGRESS / OPINIONATED INPUT.**
> Nothing in this document is a rule, a review gate, or policy. Every item
> below is a _consideration_ — an idea with its reasoning and its limits,
> offered for discussion. Quoting this document as review authority is a
> misuse. See §6 for how to disagree with it.

## 1. What this is / what this is not

**This is** the seed of a possible future Rust style guide: a readable
summary of the [Frugal Async Rust](https://worm-blossom.org/frugal_async_rust)
position and the [ufotofu](https://worm-blossom.org/ufotofu/) API philosophy
from the worm-blossom project, translated into considerations for our own
codebase. It was seeded from the #13779 auth/transport investigation, whose
worm-blossom research (p2panda-adjacent work) surfaced this material.

**This is not** an adopted position. Wholesale adoption of Frugal Async Rust
is explicitly off the table for us (see §3). What survives is local
discipline: small habits inside our existing Tokio codebase that buy
reliability without fighting the runtime.

## 2. The frugal position in one page

Frugal Async Rust argues that async Rust's incidental complexity can be
eliminated by accepting three restrictions up front:

1. **Assume that nothing is cancel-safe.** If every `.await` is a point
   where a future can be dropped and its state must still be coherent,
   writing correct async methods becomes exhausting. The frugal answer is
   to forego reliance on cancel-safety from the start: callers must drive
   every async method to completion before calling the next one.
   (Source: [Frugal Async Rust — On Cancel-Safety](https://worm-blossom.org/frugal_async_rust))
2. **Assume that no futures are `Send`.** `Send` futures can run on
   multithreaded executors, but the `Arc`/`Mutex` tax is paid by _all_
   users, even single-threaded ones. The standard library defaults to
   non-thread-safe types with threading as opt-in; async should follow the
   same logic, since async is fundamentally about concurrency, not
   parallelisation.
   (Source: [Frugal Async Rust — On Send-Bounds](https://worm-blossom.org/frugal_async_rust))
3. **Assume that panics always abort.** Unwinding, catching, and resuming
   after a panic complicates correctness (cf. `Mutex` poisoning) and
   pollutes public APIs. The frugal answer: panics must not be caught.
   (Source: [Frugal Async Rust — On Panic-Handling](https://worm-blossom.org/frugal_async_rust))

ufotofu is the companion library that lives by these principles — none of
its futures are cancel-safe, none are `Send`, and no part of it is
exception-safe — and builds a distinctive API philosophy on top:

- **Producer/consumer duality.** Adding to one end of a sequence and
  removing from the other are dual operations, so the `Producer` and
  `Consumer` traits mirror each other. Every feature of one API should
  have a counterpart in the other; asymmetry is treated as a design smell.
- **Typed final values.** A sequence is an arbitrary number of repeated
  items terminated by _at most one final value_ of an arbitrary type.
  `Final = ()` marks a finite sequence; `Final = !` (the empty type) marks
  an infinite one, statically. A well-chosen final type solves real
  problems — e.g. `query_and_subscribe` returning "all current books, then
  a producer of updates" as a single typed sequence.
  (Sources: [ufotofu design choices](https://worm-blossom.org/ufotofu/);
  [crate docs](https://docs.rs/ufotofu))
- **Explicit buffering with caller-owned storage.** Consumers may delay
  side-effects until `flush`; producers may eagerly prepare work on
  `slurp`. Buffering is abstracted in the traits, and the backing storage
  comes from a caller-provided `Queue` — no hidden allocation.
- **Zero-copy bulk processing via exposed internal buffers.** Instead of
  the `io::Read`/`io::Write` pattern (caller supplies a buffer, bytes get
  copied twice), bulk traits _expose the processor's own internal buffers_
  to an async closure. This design also statically excludes "false" bulk
  processors that loop per-item behind a bulk API.
- **All errors fatal.** After an error, no further production or
  consumption is allowed. Non-fatal problems travel as `Result` _items_,
  not errors. Fatal-only errors keep consumers from silently becoming
  coroutines and maximise what the library can guarantee.
- **`no_std`, no hidden allocation.** ufotofu 0.12.7 added Tokio compat
  wrappers at the edges, but the core stays allocation-free and
  single-threaded.
  (Source: [ufotofu crate docs](https://docs.rs/ufotofu))

## 3. Where this conflicts with our standing choices

Stated plainly, so nobody has to guess:

- Our fleet runs **multi-threaded Tokio with `Send` futures**. A blanket
  no-`Send` rule would mean abandoning the runtime we deploy on.
- We rely on **deliberate task cancellation** (timeouts, shutdown,
  supervision). A blanket "callers must run everything to completion"
  rule would mean abandoning cancellation as a control mechanism.
- We contain panics **at task boundaries** with supervisors that discard
  the failed task's state. "Panics must not be caught" as a global rule
  would mean abandoning that containment.

The source project's own history backs this off-ramp: their Rocket
experiment failed on multi-threaded runtimes (diary 2026-W42) and they hit
`Send` friction integrating with axum (diary 2026-W45). The frugal stance
is coherent _as a whole-stack choice_; grafted piecemeal onto a
multi-threaded Tokio codebase it becomes friction without the payoff.

So: **no wholesale adoption.** What follows (§4) is deliberately scoped to
local discipline that survives inside a Tokio codebase — and each item says
where it does _not_ apply.

## 4. Nine considerations

Each item states its reasoning, where it applies, where it does not, and a
source link. All nine are suggestions, not requirements.

### 1. Make cancel-safety a stated property of every async API

_Reasoning._ Cancel-safety bugs are silent state corruption: a dropped
future leaves an object mid-transition, and the next caller inherits the
mess. Most of these bugs come not from malice but from the property being
unstated — nobody documented whether dropping is safe, so callers guess.

_Applies to:_ any async method on shared or long-lived state (broker
queues, connection pools, caches). One doc line — "cancel-safe: yes/no,
because …" — costs nothing and kills a whole class of guessing.

_Does not apply to:_ fire-and-forget leaf futures that own all their state
(dropping them drops everything, so there is nothing to corrupt).

_Source:_ [Frugal Async Rust — On Cancel-Safety](https://worm-blossom.org/frugal_async_rust)

### 2. Treat `Send` as a cost decision, not a default

_Reasoning._ Every `Send` bound prices in synchronisation machinery that
single-threaded or sharded users pay for without benefit. The std
precedent — non-thread-safe by default, threading as opt-in optimisation —
exists for a reason.

_Applies to:_ leaf libraries, codecs, and per-connection state machines
that could run on a single thread. Ask "who actually moves this across
threads?" before adding the bound.

_Does not apply to:_ the brokers and shared infrastructure that genuinely
run multi-threaded. **Do not retrofit** existing `Send` code to remove
bounds; the churn would dwarf any gain.

_Source:_ [Frugal Async Rust — On Send-Bounds](https://worm-blossom.org/frugal_async_rust)

### 3. Let stream termination carry information

_Reasoning._ A stream that just _ends_ forces out-of-band signalling for
"what was the outcome?" A typed final value (or a terminal enum variant)
answers that question in-band, and `Final = !` documents infinitude so
callers stop writing accidental `while-let` loops that can never exit.

_Applies to:_ snapshot-then-subscribe patterns, paginated scans with a
completion summary, any protocol where "done, and here's the receipt"
matters.

_Does not apply to:_ plain `Stream`s consumed by generic combinators —
retrofitting final values onto `Stream`-shaped APIs fights the ecosystem.

_Source:_ [ufotofu — Generic final values](https://worm-blossom.org/ufotofu/)

### 4. Make buffering explicit and caller-owned in hot paths

_Reasoning._ Implicit internal buffers hide allocation and make flush
points unpredictable. `flush`/`slurp`-shaped commit points plus
caller-owned buffer storage make the memory behaviour reviewable: you can
see _where_ the queue lives and _when_ it drains.

_Applies to:_ hot ingestion/egress paths, batching writers, any loop where
allocation shows up in a profile.

_Does not apply to:_ cold paths and one-shot code, where an internal
`Vec` is simply fine.

_Source:_ [ufotofu — Buffering is abstracted-over in traits](https://worm-blossom.org/ufotofu/)

### 5. Prefer zero-copy bulk shapes where a benchmark proves the copy cost

_Reasoning._ Slice-at-a-time processing beats item-at-a-time on modern
hardware — but only when the copy is actually the bottleneck. The
ufotofu lesson worth stealing is narrower: _expose your own buffers_
rather than asking callers to supply scratch space, and let the API
exclude fake bulk implementations that loop per-item internally.

_Applies to:_ codec and framing layers with profiled copy overhead.

_Does not apply to:_ everywhere else — per-item processing stays the
default. No speculative bulk APIs without a benchmark.

_Source:_ [ufotofu — Zero-copy bulk processing](https://worm-blossom.org/ufotofu/)

### 6. Keep stream errors fatal and explicit

_Reasoning._ If a stream can "error and continue," every consumer must
implement its own resynchronisation policy, and most won't. Fatal errors
plus `Result`-as-item for recoverable per-item problems keeps the contract
small: after an error the stream is over, full stop.

_Applies to:_ connection- and session-scoped streams, where an error
genuinely invalidates what follows.

_Does not apply to:_ multiplexed or best-effort feeds where per-item
failure is the normal case — there, `Result` items _are_ the design, from
the start.

_Source:_ [ufotofu — All errors are fatal errors](https://worm-blossom.org/ufotofu/)

### 7. Shard by thread before reaching for shared state

_Reasoning._ The cheapest synchronisation is the kind you don't need.
`current_thread`-sharded state inside a Tokio process, bridged at the edges
(ufotofu 0.12.7's Tokio wrappers show the shape), removes contention
without removing Tokio.

_Applies to:_ per-connection or per-key state with no cross-shard
invariants — counters, caches, codec state.

_Does not apply to:_ genuinely shared coordination state (leader
election, global registries). Sharding what must agree is a correctness
bug, not an optimisation.

_Sources:_ [Frugal Async Rust](https://worm-blossom.org/frugal_async_rust);
[ufotofu crate docs](https://docs.rs/ufotofu)

### 8. Do not design APIs that need panic recovery

_Reasoning._ `catch_unwind` at arbitrary points creates half-moved state
that the type system cannot see. Treat panic as crash-plus-supervisor:
catch only at task boundaries, and discard the failed task's state
wholesale instead of trying to resume inside it.

_Applies to:_ new APIs — design them so a panic anywhere inside leaves
nothing the caller is tempted to reuse.

_Does not apply to:_ the existing task-boundary supervision, which already
follows this shape and should be left alone.

_Source:_ [Frugal Async Rust — On Panic-Handling](https://worm-blossom.org/frugal_async_rust)

### 9. Hold leaf crates to a no-hidden-allocation standard

_Reasoning._ Allocation in a leaf crate is a tax on every caller, and
hidden allocation (interior `Vec`s, boxed callbacks, implicit clones) is
the hardest kind to budget. `no_std`-compatible, caller-provides-storage
design keeps leaves honest.

_Applies to:_ small shared leaf crates (parsing, framing, crypto
primitives) where the dependency fan-out makes the tax real.

_Does not apply to:_ binaries and services, which own their runtime and
may allocate freely.

_Source:_ [ufotofu crate docs](https://docs.rs/ufotofu) (`#![no_std]`)

## 5. How this document may evolve

- It grows by **dated additions**, not rewrites: new material lands as new
  dated records that cite this one.
- Promotion to a real style guide is **possible but not assumed**. That
  would be a separate proposal with its own review, not an automatic
  graduation.
- Quoting this document as review authority is a **misuse**. If an item
  here ever becomes enforceable, the enforcement lives in the style guide
  or CI check that adopts it — cite that, not this.

## 6. How to disagree with this document

Disagreement is expected; this record is opinionated input, not consensus.
To disagree productively:

1. Write a dated follow-on record (or a section in the issue thread) that
   names the specific consideration and the counter-evidence.
2. Prefer measurements over assertions — a benchmark beats a paragraph.
3. If the disagreement is with the _sources_ (e.g. the frugal position
   itself), say so; this record is a translation, and a flaw in the
   original is worth naming rather than laundering.

## 7. Sources

- [Frugal Async Rust](https://worm-blossom.org/frugal_async_rust) —
  the three assumptions (cancel-safety, `Send`, panics) and their
  reasoning. Read in full 2026-11-23.
- [Ufotofu design notes](https://worm-blossom.org/ufotofu/) —
  producer/consumer duality, typed final values, explicit buffering,
  zero-copy bulk processing, fatal-only errors. Read in full 2026-11-23.
- [ufotofu crate docs](https://docs.rs/ufotofu) — futures are `!Send`,
  not cancel-safe, not exception-safe, `#![no_std]`; 0.12.7 added Tokio
  compat wrappers. Read 2026-11-23.
- Worm-blossom diary via the #13779 investigation research: Rocket
  experiment failing on multi-threaded runtimes (2026-W42); axum
  `Send`-trait friction (2026-W45).
