# The shape of the system

The PM stack has six parts. One reads, one links, one launches, one set does the work, one is read on a screen, and one is carried in a pocket. This chapter is the whole system on one page; each part has its own chapter later in the book.

## The six parts

| Part       | What it is                                                                                                                                                                           | Where it runs                                        |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------- |
| Aggregator | The dashboard service. It reads every source, publishes one state as pages and machine twins, raises questions, and records answers and actions                                      | Laptop host `amd`, port 4870                         |
| Projector  | The link registry and projection service. It records which items in different sources are the same work, and serves links and latest comments per subject                            | Laptop, port 7801                                    |
| Dispatch   | pm-dispatch, the intake service. An authenticated request launches a session now or enqueues it as a queue task with a tracking issue                                                | Laptop, port 4879, public as `dispatch.security.cab` |
| Workers    | The things that do and watch the work: OpenCode sessions claimed by the claimant, the fleet queue's lanes, Cloudflare Workers at the edge, and the guard and exporter jobs on timers | Laptop, GitHub Actions, and Cloudflare               |
| Website    | Two first-party web apps: the aggregator dashboard (29 routes) and the PM console (21 hash views)                                                                                    | Laptop, with the console also served at the edge     |
| Phone      | The console as an Android app, plus Web Push to subscribed devices. The phone reads the same state and answers through the same action table                                         | Drew's phone                                         |

## How a piece of work moves through the shape

1. Work exists first in a source: a GitHub issue, a Kaneo card, a Grist roadmap row, a roadmap file in a repository, or a queue task. The stack creates none of these records by reading them.
2. The projector links the records that are the same work. A webhook from GitHub (`POST /github/webhook`, HMAC-checked, deduplicated by delivery GUID) and reconcile polls keep the links current. The projector never writes back to GitHub, Kaneo, Pad, or Grist, and never becomes a fifth source of truth.
3. The aggregator reads the sources and the projector, and publishes the joined state: board, roadmap, program, queue, needs-you, quota, and health. Where two sources disagree, both values are kept and the disagreement is listed on `/conflicts`.
4. Work is launched through dispatch or claimed from the queue. Dispatch creates the tracking issue, enqueues a `repo-task/v1` queue task, creates an OpenCode session on the laptop server, and claims the task for `dispatch-opencode` with a 6-hour lease, in that order, after a placement guard checks the named model pool is not walled.
5. When work needs Drew, a question is raised on one of the eight question surfaces and a push is sent. Drew answers on the website or the phone. The answer is recorded, applied to the ledger or performed as an audited action, and the item's next published state carries the result.

## The rules the shape enforces

- **Reading is not writing.** The aggregator reads Kaneo read-only, reads Grist and the roadmap files, and reads the Project pool through the fleet feed. It does not edit Project fields and does not write GitHub labels (D4). The one exception inside the website is deliberate: dashboard comments and roadmap annotations are stored in the aggregator's own stores, and anything that must change a source becomes an action for the writer that owns that source.
- **One writer per record.** The orchestrator is the single writer of GitHub Project fields. The orchestration writer writes orchestration state to the shared store in one transaction per pass. See [The shared schema](../concepts/shared-schema.md).
- **Pad is planning only.** The aggregator reads no Pad planning data; no source adapter consumes Pad content. It does probe Pad for health and topology, and Pad is a valid stack-token target (D3). Pad sits in the shape as a planning surface for people, not as a data source for the aggregator.
- **The console is hand-installed, not NixOS-declared.** `pm-console` is the one PM-stack app the NixOS module does not declare: it runs as the hand-installed user unit `pm-console.service` on port 4872, installed by `pm-stack/console/install.sh` and kept current by its own per-minute redeploy timer. See [Installation and bootstrap](installation.md).
- **The edge holds, the laptop decides.** When the laptop is unreachable, the console edge Worker serves the console and holds actions and answers. Held work replays through the same action table when the host returns. The notify relay that would extend this to push answering at the edge is not deployed, and its answer routes are not routed (D5).
- **The aggregator makes no model calls.** The aggregator, console, dispatch, and projector make no LLM calls and no embedding calls of their own. Models enter the shape as things the stack routes work to and measures — dispatch pins model hints to quota pools, the quota pages measure pools — and in two adjacent components: the voice-line Worker's speech inference and the Atlas embedding dependency, which belongs to Atlas, not to this stack.

## Where each part is covered

[Architecture](../concepts/architecture.md) groups all 36 surveyed components under this shape. [The aggregator: one state, many sources](../concepts/aggregator.md) covers the reader. [The projector and projections](../concepts/projector.md) covers the linker. [Installation and bootstrap](installation.md) covers where each part runs and how it gets there.

Sources. Survey files `~/workspace/pm-book/survey/apps.md` (§1–§14, deployment frame), `~/workspace/pm-book/survey/website-pages.md` (introduction, §E–§F), `~/workspace/pm-book/survey/disagreements.md` (D3, D4, D5); repo paths `pm-stack/README.md`, `pm-stack/PORTS.md`, `pm-stack/projector/README.md`, `pm-stack/dispatch/README.md`, `pm-stack/aggregator/src/program.ts`.
