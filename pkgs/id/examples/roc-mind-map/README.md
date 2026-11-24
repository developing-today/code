# Roc Mind Map Example

A playable mind-map world for `mind-map/same-label-distinct-nodes` (#13609).
Nodes are distinguished by stable ID, so two nodes can share the label
"Archive". A typed cross-link (`related`) keeps pointing at its node through
reparent and rename. A reparent that would put a node under its own descendant
is refused and leaves the tree unchanged. Positions never change.

Inputs are JSON commands:

- `{"kind":"AddCrossLink","source":"U","target":"V"}` links U to V (`linked`).
- `{"kind":"Reparent","id":"U","new_parent":"Q"}` moves U under Q (`moved`).
- `{"kind":"Rename","id":"U","name":"Inbox"}` relabels U (`renamed`).
- `{"kind":"Reparent","id":"Q","new_parent":"U"}` is refused (`refused-cycle`):
  Q would land under its own descendant U, so the tree is unchanged.

`records` publishes the domain state (`root`, `nodes` with `id`/`name`/`parent`/`position`,
`cross_links`, and the last `outcome`); `snapshot` is the same document, so a
snapshot restored into a fresh instance compares byte-identical through the
regular host. The view draws the nodes on a portable screen with the last
outcome and the link list.

## Build

```bash
./build.sh
```

Produces `mind-map.wasm`, validated as an import-free Wasm module.

## Run

```bash
id serve --data-dir ./data --world --world-admin-token "$ADMIN" \
  --world-module "$PWD/examples/roc-mind-map/mind-map.wasm" \
  --world-ssh-port 2223 --web --port 8787
```

Join with SSH or the web page and send an input such as
`{"kind":"Rename","id":"U","name":"Inbox"}` to relabel U.
