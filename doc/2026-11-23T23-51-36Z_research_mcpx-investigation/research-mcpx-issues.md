# mcpx — issue census, blocker chains & Drew-gated items (research, 2026-11-23)

Source: GitHub Search API + REST reads against developing-today/code, 2026-11-23
~06:35 CST. Counts are label-exact from the Search API `total_count`; the
programme's precise union (labels overlap) is the issue-map deliverable.

## 1. Census headline

| Slice | Open | All-state |
|---|---|---|
| `app/mcpx` | **21** | 374 |
| `epic/mcpx-platform` | **45** | 110 |
| `epic/mcpx` | 3 | — |
| `epic/mcpx-maintenance` | 1 (the epic itself, #12459) | — |
| `queue/blocked` ∩ `app/mcpx` | 4 | — |
| `status/blocked` ∩ text "mcpx" | 18 | — |
| Any open issue mentioning "mcpx" (title/body) | 92 | — |

The two main labels overlap heavily (most `app/mcpx` issues also carry
`epic/mcpx-platform`); the tracker family (#1884–#1987) mostly carries only
`epic/mcpx-platform`. Programme homes:

- **#8993** — epic: one canonical MCP gateway and server parity. Children by
  label: #8822 (dedup — shipped, tracking home) + #8809 (server logins).
  Both are natively parented under the design-register epic #8986 (one-parent
  rule), so #8993 has 0 direct sub-issues by design, recorded on the epic.
- **#12459** — epic(mcpx): maintenance (parent: #8993). Recurring
  implementation/reliability/docs/hygiene follow-ups live here.
- **#1900** — the backlog build-out super tracker: every theme, in one order,
  with the disagreements between them.
- **#8822** — dedup shipped; remains open as the tracking home.

## 2. The open `app/mcpx` set (21, complete)

12459 (maintenance epic) · 11147 (two unbuilt #1962 considerations:
adapters.allowBare gate + cwd for bare tools; in-review; gate half also #13118) ·
10471 (CI over budget, rolling) · 9963 (#1384 Phase 3: supervised stdio by
default) · 8473 / 8472 / 8471 / 8469 / 8465 (coverage burn-downs) · 7843
(user-identity conformance design) · 4375 (bounded response-read contract
design) · 3016 (nightly stress failure; blocked) · 2841 (#1858 SET-REMAINDER;
blocked) · 2686 (e2e/settings suites vs nix sandbox; blocked) · 1987 / 1985 /
1983 (testing platforms / CI skeleton / environments) · 1957 (smaller
conformance gaps) · 1940 (code mode interop) · 1414 (session-resume design) ·
1384 (supervisor design).

The remaining ~24+ `epic/mcpx-platform`-only opens are the tracker family
(#1884–#1929), the docs trackers (#1888–#1894, #1943), embedding (#1877),
plugin core (#1861), v2 plugin (#1858), OAuth (#1950, in-review), model layer
(#1925, blocked), simulation (#8242), release channels (#1881, #9900 blocked),
failover (#8017, blocked), and the design-mill passes (#10389–#10405).

## 3. Blocker chains (heads first)

- **Config-system completion**: #8473 (proto family) → its last exemption is
  `protoTasks.cancelGrace` — **flag-held**; #8472 (plugin family) → remainders
  `plugin.instructions` + `plugin.disabledTools` — **flag-held** (the 11 sibling
  plugin.* exemptions were cleared 2026-11-23 by JS-host e2e probes; these two
  wait on Drew's word). The unheld five (`daemon.inlineStartPoll`,
  `daemon.tsnet.hostname`, `daemon.tsnetUpTimeout`, `http.streamBufferInit`,
  `embeddings.wasmPath`) have no chain — they wait on harness shapes, not issues.
- **Plugin line**: #1858 (opencode v2 plugin rewrite) → #2841 (SET-REMAINDER
  items 1–5, blocked). #1861 (harness-free plugin core) is the parallel design.
- **Test-venue line**: #2686 (nix build sandbox can't run e2e/settings suites)
  and #10330 (three scripts suites, no registry-named venue) gate how much of
  the coverage programme CI can even see; #12738 (in-review: `nix build .#deploy`
  wedges in the mcpx check phase) is the same family from the deploy side.
- **Release line**: #9900 (GHCR container channel never published — push denied
  on the v0.1.0 run; publish path now dispatch-only; blocked) → #1881
  (release.yaml, artifacts, the install channels the docs promise).
- **Atlas line**: #8625 (Atlas MCP in mcpx — the wiring is LIVE on the laptop;
  the remainder is cross-client acceptance, blocked) — its blockers live in the
  Atlas programme (D1 chain etc.), not in mcpx.
- **Failover line**: #8017 (github failover: daemon call-path wiring + live
  activation, follow-on to #7948; blocked).
- **CI health**: #3016 (nightly stress failure, blocked), #10471 (CI over
  budget, rolling), #8866 (PR workflow runs cancelled en masse; one mcpx run
  stuck queued; blocked, user/escalated).
- **Pages line**: #13225 (stage fleet docs into the writable Pages directory) —
  the mcpx-docs deploy now carries the fleet book and will carry atlas/id;
  staging correctness is mcpx-deploy maintenance.

## 4. Waiting on Drew (decisions / gates — surface, never decide)

- **#8809** — GitHub / Atlassian / Datadog MCP server logins (OAuth/PAT):
  `user/escalated`, blocked. Epic #8993's parity half cannot finish without it.
- **#8810** — one-command local agent CLI via this VM's mcpx: needs Drew's
  OS / call-method / auth answers. `user/escalated`, blocked.
- **#1943** — mcpx docs custom subdomain: the recorded pending decision is
  Drew's "apply it" for the mcpx CNAME.
- **Flag holds touching mcpx** (Drew's item-specific approvals, recorded
  2026-11-23): #8472's `plugin.disabledTools` and `plugin.instructions`
  remainders; #8473's `protoTasks.cancelGrace` (last exemption in #8473).
  Investigation and design around these is free; implementation is not.
- **#12900 thread 2** (Atlas programme) — is Atlas headed for multi-client
  use? The answer shapes mcpx's per-client ceiling design; it is Drew's call,
  prepared as an options paper by the Atlas programme.

## 5. Cross-programme dependencies

- **Atlas**: #8625 (above); #6062/#9109 Part C (ID graph publication) reaches
  mcpx when ID content becomes searchable — mcpx is the likely serving gateway.
- **ID**: #7916/#7917 (MCP bridge + PolicySession) closed 2026-11-23 — ID's
  capability-first MCP posture is landed on the ID side; the interop design
  (ID as mcpx upstream; mcpx as ID's gateway) is unfiled design space.
- **fleet**: design-mill #10389/#10390/#10393/#10405 (all open, triaged);
  #13191 (queue repair verb) is fleet-side but its book chapter rides the
  fleet book's DOC-01 rule — the same docs-sync pattern this programme extends
  to mcpx.
- **OpenChamber cascade**: services.openchamber.enable → opencode + mcpx
  (2026-11-04 consolidation) — mcpx's module surface is consumed via the
  openchamber flake, so module changes have a cross-repo consumer.
