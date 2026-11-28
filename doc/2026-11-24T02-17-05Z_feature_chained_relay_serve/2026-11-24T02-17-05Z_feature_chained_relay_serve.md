# Chained-relay serving in `id serve`

Issue #5534, design #2036, acceptance 2/3. The requested slice registers
`CHAINED_RELAY_ALPN` in the production `id serve` router, with the relay gate
backed by the same signed-graph event store used by `LocalEvents`. Since that
store is opened only with the web event store, relay serving is conditional on
those inputs being available. The graph frontier fails closed, so unauthorized
and unresolvable membership traffic is rejected before the forwarding sink.

The serve layer does not currently expose a neighbour-discovery view suitable
for relay fan-out. Use the production forwarding sink with an empty neighbour
set for this slice; later neighbour-discovery work can supply a live peer set.
This does not broaden the umbrella scope or close #5534.
