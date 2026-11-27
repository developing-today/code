# fleet host

`fleet host <name>` resolves a host name through the machine
registry and selects a transport for it (#14716, transport & pooling
design §1.3). It is the N-host generalisation of the laptop route
forms: where [fleet laptop](laptop-route-forms.md) walks the tiers
of one known host, `fleet host` starts from a name, builds the
candidate set from the registry, and lets the fleet-pool primitive
(#14721) choose — selection is a pool decision, never a hand-written
per-host ladder, and every result names the serving offer and form.

```
fleet host <name> [--class <agent-class>]
```

Argument handling is strict: a name, an optional `--class`,
`--help`, and nothing else (exit 2 on anything unaccounted for).

## The registry as data

Host records mirror the reviewed registry artifact
`doc/fleet-machine-registry.yaml` (schema slice 1a, #14714): registry
`id`, human names, transport offers (endpoint data and credential
**names** only — never values), and the enrollment table saying
which agent classes may use which offers. A reviewed edit to the
YAML is the source of truth; the compiled-in table in the binary is
its read path. The one record today is `amd-6` (names `amd`,
`amd-6`); the legacy name `laptop` resolves to it, exactly as the
tiered `fleet laptop` walker serves that host.

The amd record's offers are `commander-door`, `laptop-agent-cf`,
`laptop-agent-tailnet` and `ssh`. Enrollment: `opencode-lanes` and
`muse-seats` may use the full ladder; `external-mcp-clients` may use
`commander-door` only. A class with no enrollment row may use no
offers. The default class for a bare `fleet host <name>` is
`muse-seats` (this binary's seats are muse seats unless a caller
names a class).

## Candidates and selection

Resolving a name yields one candidate member per (transport offer
the class is enrolled for × invocation form of the offer's kind) —
the #13717 matrix as data. Form ids per kind: `commander-door` →
T1/T4, `laptop-agent` and `tsnet` → T2/T5, `ssh` → T3/T6. (The
tiered T0 and the wrapper T7 are walk shapes over the whole
candidate set, not per-offer candidates, so they never appear here.)

The broker live overlay (1b, #14715) feeds fresh per-offer
health/load into selection when available: stale or unhealthy offers
are excluded, and measured load is consumed by the pool's
least-loaded strategy. Missing load remains unknown — never zero
(#13611) — and the strategy's unknown-last rule applies.

## Output

```
host: laptop (amd-6)
* T2 via laptop-agent-cf (laptop-agent)
  …
selected: T2 via laptop-agent-cf (laptop-agent)
decision: member=… strategy=… reason=…
```

Every candidate is printed (the selected one starred), then the
selection, then the pool's decision log read back — member,
strategy, reason. A pool decision you cannot read back is a bug.

Exit codes: 0 ok · 1 no candidates (unknown host, or the class is
enrolled for no offers) · 2 usage.
