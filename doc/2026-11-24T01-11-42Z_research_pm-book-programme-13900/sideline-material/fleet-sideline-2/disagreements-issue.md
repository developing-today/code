# Fleet book (fleet-gaps additions): three disagreements found while surveying, D36–D38

Filed by lane FLEET-SIDELINE-2, programme #13900, 2026-11-23, per the
bookwright rule: disagreements between docs/designs and code, grouped
per surface. Numbering continues the fleet-gaps survey's D30–D35.
Where a doc and the code disagree, the code wins, and the new fleet
book chapters follow the code.

## D36. The fleet book links to a Lab Book that has not landed

`pkgs/fleet/book/development/high-level-roadmap.md` links to the Lab
Book's roadmap page at `developing-today.github.io/code/lab/high-level-roadmap.html`.
Checked 2026-11-23: no Lab Book sources exist in this repo (no lab
book package beside `pkgs/code-book`), and the combined Pages deploy
(`.github/workflows/mcpx-docs.yml`) stages the mcpx, fleet, atlas, id
and code books — no lab stage. The Lab Book is a deliverable of
programme #13905 whose sources and chapter map are still being
settled by its lane. The link anticipates the book; until it lands,
the target is a dead end. The new cross-links chapter
(`concepts/lab-cross-links.md`, PR for this lane) therefore links the
Code Book's landed "The Lab" chapter and names #13905 as the owning
programme instead of linking the unbuilt book.

## D37. #13774's premise describes a Worker-gated exec door; the landed door has no Worker

The #13774 issue body (premise, "Rendezvous constraint") describes
the Actions exec public door as "a public door in the commander/mcpx
pattern (Worker gate + tunnel → laptop)". The landed door, recorded
in the same issue's Slice 5 comment (2026-11-23T22:10:06Z) and settled
independently by programme #13900's WORKER-EVIDENCE survey, is a
path-scoped Cloudflare Tunnel ingress only: `exec.security.cab` with
path `^/actions-exec-(register|poll|io|result|close)$` forwarded to
the laptop queue broker, no Worker, no Access app — the runner job's
GitHub OIDC token is the authentication, verified at the broker.
There is no wrangler config for an exec door anywhere in the repo,
and in code the door is only a broker URL constant
(`pkgs/fleet/crates/fleet/src/actions.rs`:
`DEFAULT_BROKER_URL = "https://exec.security.cab"`). The new chapter
(`concepts/temporary-builds.md`) follows the landed state.

## D38. The task-21517 routing packet names a wrong crate path

The #13765 comment record routing the coordinator build as queue
task 21517 (lane code-i, 2026-11-23T21:19:22Z) records that the
packet's path was wrong: fleet-queue lives at
`pkgs/fleet/crates/fleet-queue`, not `crates/fleet-queue` as the
packet said. Recorded here so the dedicated slice-1 task the parking
record calls for is not cut from the same packet text. The new
chapter (`concepts/compile-coordinator.md`) uses the verified path.

— Lane FLEET-SIDELINE-2, programme #13900, 2026-11-23.
