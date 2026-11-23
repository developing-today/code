# id-hw-shell

ESP32-class hardware shell spike for `id` — more-shells v1 slice 3
(developing-today/code#10929; design
`doc/2026-11-10T20-40-00Z_design_more_shells_v1`, §3).

The shell speaks the F13.1 shell contract
(`pkgs/id/contract/shell-contract.v1.json`) in the subset the
design says survives at this device size:

- **presence** — who/what is in the world, folded from the
  journal's system joined/left events;
- **journal-head state in compressed form** — the newest durable
  sequence, and nothing else about the journal;
- **confirm/act** — answering a presented form choice with a
  `form_result` frame (submit/cancel, choice selections, yes/no
  confirmations).

Excluded by name (see `capability_statement()` in `src/lib.rs`,
which the tests pin to the contract file and to the code):
rich rendering, free text entry, world hosting, provider offers.
Posture is wake-on-journal-head: no live session is held; resume
joins with `after`, and a non-contiguous replay marks the shell
diverged and discards folded presence until a fresh snapshot.

The crate is transport-free — pure frame builders and a fold — so
a device build pairs it with its own iroh (or paired-node)
transport. Tests:

- unit tests in `src/lib.rs`: the capability statement against
  the contract and the builders, the presence/head fold, resume
  divergence, and the confirm/refusal rules;
- `tests/live_wake.rs`: the behavioral check against a
  self-hosted world (in-process host): wake → presence + journal
  head → presented choice → confirm → the program records the
  result.

Run inside the pkgs/id dev shell:

```sh
cargo test -p id-hw-shell
```
