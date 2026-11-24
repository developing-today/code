# ID-5814: Adapter conformance harness and interop matrix

Status: proposed (design record for #5814, parent #5674, programme epic #2056).
Filed by lane code-j. Settles the shape of the conformance harness and the CI
interop matrix. It does not re-decide the wire schema or adapter model of #5674,
and it does not change the messaging semantics of #2036.

## Decision

1. **Adapters are certified by a shared harness, not by review.** Every adapter
   crate implements one trait-level contract (encode/decode a frame over its
   carrier, report its capabilities and MTU) and is run through one conformance
   suite. An adapter is "supported" only when its suite result is recorded in CI.
2. **Golden vectors are the oracle for the wire, property tests are the oracle
   for the carrier.** The postcard schema is checked against published
   byte-for-byte vectors per message type and schema version (the vectors are
   the spec, so a non-Rust implementation can check itself against them). Each
   adapter is checked by round-trip property tests: an arbitrary valid payload
   crosses adapter then decoder unchanged.
3. **The interop matrix is generated, not hand-written.** The harness enumerates
   adapter pairs (A client, B peer, optionally via a relay) and runs one
   canonical session per pair. A new adapter is added to the matrix by
   registering it, and CI fails if a registered adapter has no row.
4. **Negative behaviour is part of conformance.** Truncated, oversized,
   unknown-version and unknown-type frames must produce the specified error
   class. Never a panic, never a hang. Fuzzing with a bounded budget runs as
   a nightly, not in the merge gate.
5. **Performance baselines are recorded per adapter** as a throughput and
   latency envelope. A regression beyond the envelope fails the nightly; the
   envelope is measured on the laptop executor, not in shared CI, so the number
   is stable.
6. **Test placement.** Harness code lives in the `id` package under a conformance
   module. Each new test file is registered in `.github/test-suite-registry.json`
   so it has a CI venue (see the feature-slice skill, step 5).

## Invariants implementation must hold

- I1. Adapters carry frames unchanged. An adapter never inspects or rewrites
  the payload, so an adapter compromise cannot forge identity (#5674 §6).
- I2. Decode of any byte string terminates in bounded time and returns either a
  frame or a named error.
- I3. Two adapters that both claim a schema version settle on the highest
  common version and never silently downgrade (#5674 §2).
- I4. Fragmentation and reassembly are deterministic at MTU boundaries, and
  duplicate or out-of-order delivery is tolerated only by adapters that declare
  store-and-forward capability.
- I5. A passing matrix cell is evidence for that pair only. It is not evidence
  for a third adapter, and the matrix must not infer transitivity.

## Alternatives rejected

- **Per-adapter ad-hoc tests.** Each adapter would invent its own pass criteria,
  which is what #5674 exists to prevent. Rejected.
- **Certification by a single reference implementation's output.** This turns
  one implementation into the spec and hides its bugs. Golden vectors are
  published separately so the reference is checked too. Rejected.
- **Full pairwise matrix on every PR.** Quadratic in adapters and too slow for
  the merge gate. The merge gate runs a smoke subset (one peer per adapter
  class); the full matrix runs nightly. Chosen split.
- **A single winning carrier.** Explicitly a non-goal of #5674. Rejected.

## Open points, recorded with defaults

- Which adapter is the smoke-subset representative for each carrier class.
  Default: the native iroh/UDP adapter plus one stream and one store-and-forward
  adapter, chosen when the first adapter lands.
- Whether vectors live in `pkgs/id/test-vectors` (exists today) or a new
  directory. Default: reuse `pkgs/id/test-vectors`.

## Decomposition (for follow-up issues, not filed here)

Implementation is a separate decision, possibly months out. When it starts, the
first slice is the harness trait plus the golden-vector runner against the
native adapter. After that it is the negative-frame suite, then the matrix
generator, then per-adapter envelopes.

## Evidence level

Design only. Nothing here was built or run. The file references were checked
against `origin/main` at 3273155a115 by reading the parent issue body and the
existing `pkgs/id/test-vectors` directory listing.
