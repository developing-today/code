# 10x reference-laptop stress run

Run date: 2026-11-25. Host, toolchain, and source revision are recorded in
[`machine.txt`](machine.txt); generator details and the 256-participant cap
wall are in [`fixture/manifest.json`](fixture/manifest.json). Raw command
output is kept in the adjacent profile logs. The benchmark executable was
built from source revision `f59f94f7ee8b895b8b2f1d48f2045be50e543cc9` and
run on the reference laptop (AMD Ryzen 7 7840U, 16 threads, NixOS).

## Results

- `stress_gen`: 11,001 worlds (10,000 idle, 1,000 active, one hot), 200,000
  events, seed 22673, hot-state hash `c7c4fb2805ee72ed`; one expected wall:
  the 320-member tier target was generated at the unchanged 256-member cap.
- `stress_mem`: five fresh sequential processes, no walls. Median RSS deltas
  per world: 8,039 B idle, 87,074 B active; hot-world delta 20,815,872 B.
  Median peak RSS 271,589,376 B. Compaction spikes: 5.17 MiB at 2,000
  entries and 89.36 MiB at 100,000 entries.
- `stress_join`: five fresh processes; each validated the hot world at a
  checkpoint boundary and mid-interval and joined all 1,000 active worlds.
  Medians of the five process-level medians (µs/join): cold hot-world 3,245
  boundary / 5,199 mid-interval; warm hot-world 465 / 464; cold active-world
  106 / 124; warm active-world 14.7 / 14.5.
- `frame_cost`: five fresh processes at 10k / 100k / 1M applied events.
  Fuel remained 958,616 (view) / 958,683 (layout) at each size. Median view /
  layout wall time was 75.7 / 76.3 µs, 79.3 / 79.1 µs, and 76.7 / 76.7 µs.
- `many_worlds`: wall after 89.76 seconds with `server has too many worlds
  open; try again shortly`; this is recorded as a measured wall, not a pass.

## Reproduction

From `pkgs/id/` on the reference laptop, run the release benchmark profiles
against the same source revision:

```sh
cargo bench --bench world_bench -- --only stress_gen
for profile in stress_mem stress_join frame_cost; do
  for rep in 1 2 3 4 5; do
    target/release/deps/world_bench-<built-hash> --only "$profile"
  done
done
target/release/deps/world_bench-<built-hash> --only many_worlds
```

`stress_gen` is deterministic and was run once to create the shared fixture;
the other successful profiles were run in five fresh sequential processes.
The `many_worlds` wall was observed once. The fixture's generated world files
are intentionally not checked in; the manifest plus deterministic generator
recreates them, while the retained logs preserve the measured output.
