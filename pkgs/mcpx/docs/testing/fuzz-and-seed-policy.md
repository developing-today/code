# mcpx fuzz and seed policy (#1960)

This is the working policy for deterministic generated tests and Go's native
fuzzing. The two systems complement one another; they do not share a random
stream.

## Seeded generated tests

- Test data generated from randomness must use `internal/testsupport/gen` and
  one explicit run seed. Do not use package-global `math/rand`, `time.Now()`
  as a seed, or `crypto/rand` to create test inputs. When production code uses
  randomness, prefer injecting a source so tests can control it.
- Derive a test's source with `gen.ForTest(t, runSeed, generatorName,
  generatorVersion)` (or `gen.Stream` when a test name is supplied explicitly).
  A stream is keyed by the run seed, test name and generator `name@version`;
  adding another test or changing one generator must not shift unrelated
  streams.
- Generators draw only through `gen.Source` (`Uint64`). Use the range and byte
  helpers in `gen`; do not depend on the output sequence of `math/rand` or
  consume hidden draws for bookkeeping such as input length.
- Do not hard-code a normal run seed. Fixed seeds belong only in golden tests
  that pin the derivation contract. A generator output change requires a
  generator-version bump and a corresponding manifest update.
- A failing generated test must print enough context to replay it: the run
  seed, test name, generator name/version and failing case. Preserve a useful
  failure as a regression test; do not rerun until it happens to pass and drop
  the original seed.

The CI run-seed store at `testdata/seeds/v1` records run metadata collected
from `test-seed` artifacts. It is not the native fuzz corpus; the collector
workflow is its only writer. Do not hand-edit seed records.

## Native Go fuzz targets

- A fuzz target accepts arbitrary inputs and checks stable invariants. Keep it
  deterministic for the same input, bounded in memory and CPU, and independent
  of network services, wall-clock time and shared mutable state.
- Add a small set of representative `f.Add` inputs. Go replays these seeds
  during ordinary `go test`; ordinary tests do not perform mutation. Preserve
  minimized failures in `testdata/fuzz/<FuzzName>/` so the regression runs on
  every normal test invocation.
- When adapting `[]byte` to a generated structure, use
  `gen.NewBytesSource(data)`. Derive payload size from `len(data)` rather than
  consuming bytes to choose a size; this keeps the complete fuzz input
  available to the target.
- A fuzz failure is a bug report: keep the reproducer, add or strengthen the
  invariant/regression test, and record which generator or input shape could
  have found it. Do not silently discard a failing corpus entry.
- Native fuzzing uses Go's own mutation engine and does **not** consume the
  run-wide `gen` seed. Its minimized corpus is separate from `testdata/seeds`.

## Commands and CI boundary

Replay all saved corpus entries with the normal package tests:

```sh
cd pkgs/mcpx
go test ./...
```

Run the bounded PR smoke for the current proof target:

```sh
cd pkgs/mcpx
go test ./internal/config -run '^$' -fuzz '^FuzzStripJSONC$' -fuzztime=5s -parallel=1
```

The `mcpx-test` workflow runs this short smoke after the regular unit tests.
Long fuzz campaigns and soak policy remain with the separate fuzz-soak work
(#1985); do not turn the PR job into an unbounded run.
