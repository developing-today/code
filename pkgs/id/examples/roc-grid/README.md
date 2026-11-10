# Roc Grid Example

A minimal world program built on `pf.RocGrid`, the platform's bounds-checked
row-major byte grid.

The model holds a 4x3 grid filled with `.`. A `set` input writes `A` into the
centre cell when the coordinate is in bounds. The view reports the count of
untouched cells, the cell under the cursor, and an out-of-bounds read served
by its fallback.

## Build

```bash
./build.sh
```

Produces `roc-grid.wasm`, validated as an import-free Wasm module.

## Run

```bash
id serve --data-dir ./data --world --world-admin-token "$ADMIN" \
  --world-module "$PWD/examples/roc-grid/roc-grid.wasm" \
  --world-ssh-port 2223 --web --port 8787
```

Join with SSH or the web page and send the input `set` to claim the centre.
