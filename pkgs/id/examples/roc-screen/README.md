# Roc Screen Example

A minimal example demonstrating the `id` portable screen presentation protocol (`Screen.roc`) and terminal key parsing (`Key.roc`).

The program maintains a counter that increments when receiving `+` input, and renders a structured cell grid within a bordered box titled `"counter"`.

Adapters (SSH ANSI presenter, browser DOM presenter) render the structured screen contract natively without executing arbitrary markup.

## Build

```bash
./build.sh
```

Produces `roc-screen.wasm`, validated as an import-free Wasm module.

## Run

```bash
id serve --data-dir ./data --world --world-admin-token "$ADMIN" \
  --world-module "$PWD/examples/roc-screen/roc-screen.wasm" \
  --world-ssh-port 2223 --web --port 8787
```

Join with SSH or web to see the styled box counter, and send `+` to increment.
