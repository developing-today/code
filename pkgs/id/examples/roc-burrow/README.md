# Roc Burrow Example

A playable burrow world for `burrow/detour-to-exit` (#13603).
On a 4×3 board the digger starts at (0,1); soil fills (1,1), (2,1) and
(2,2), a rock blocks (3,1) and the exit waits at (3,2). Digging enters an
orthogonally adjacent soil cell and clears it exactly once. The rock is
impassable. The exit accepts entry only once three soil cells have been
cleared, so the digger detours south around the rock.

Inputs are JSON commands:

- `{"kind":"Dig","direction":"E","event":"d1"}` digs into adjacent soil
  (`cleared`), moving the digger into the cell.
- Digging toward the rock reports `blocked-rock` and changes nothing;
  digging toward the board edge reports `blocked-edge`; digging toward
  open floor is `refused`.
- `{"kind":"Walk","direction":"E","event":"w1"}` enters an already open
  cell (`moved`); uncleared soil refuses with `blocked-soil`.
- Entering the exit with three clears reports `escaped` and sets the
  phase; before three clears it reports `blocked-exit`.

The `event` tag is host journal deduplication metadata; the guest does
not read it. Host events (participant 0) are ignored.

`records` publishes the domain state (`digger`, `soil`, `cleared_count`,
`phase`, and the last `outcome`); `snapshot` is the same document, so a
snapshot restored into a fresh instance compares byte-identical through
the regular host. The view is a semantic text grid — `A` the digger,
`s` soil, `#` the rock, `X` the exit, `.` open floor — with the
remaining soil and the clear quota as text lines.

## Build

```bash
./build.sh
```

Produces `burrow.wasm`, validated as an import-free Wasm module.

## Run

```bash
id serve --data-dir ./data --world --world-admin-token "$ADMIN" \
  --world-module "$PWD/examples/roc-burrow/burrow.wasm" \
  --world-ssh-port 2223 --web --port 8787
```

Join with SSH or the web page and send an input such as
`{"kind":"Dig","direction":"E","event":"d1"}` to start digging.
