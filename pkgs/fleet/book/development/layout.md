# Repository layout

Fleet lives in `pkgs/fleet/` in the `developing-today/code` repository. It is a self-contained Cargo workspace on the `pkgs/atlas-search` pattern: its own `Cargo.toml`, `Cargo.lock`, and `rust-toolchain.toml`, with no workspace relationship to the repository root. The design of record is epic `developing-today/code#5412`.

This chapter is the map. For how the pieces are built, see [Building fleet](building.md). For how they are tested, see [Testing](testing.md).

## The 13-crate workspace

`pkgs/fleet/Cargo.toml` declares 13 members, `resolver = "2"`, workspace version `0.1.0`, edition 2024, rust-version 1.85, licence `MIT OR Apache-2.0`. Shared external dependencies are pinned once in `[workspace.dependencies]` (`serde 1` with derive, `serde_json 1`, `sha2 0.11`, `toml 1.2`); the 12 library crates are path dependencies of the binary crate.

| Crate | What lives there |
|---|---|
| `crates/fleet` | The binary crate. `[[bin]] name = "fleet"`, entry point `src/main.rs`. It has no `lib.rs`. It also holds the bin-crate-only domains: `src/secrets.rs`, and the `gh` domain in `src/gh.rs` with `src/gh_cache.rs`, `src/gh_closure.rs`, `src/gh_guard.rs`, `src/gh_metadata.rs`, plus `src/session_state.rs`, `src/invocation_ledger.rs`, `src/derived_version.rs`, and `build.rs`. |
| `crates/fleet-registry` | The full domain registry: names, one-line descriptions, argv0 aliases, capabilities, and variant profiles. Standard-library only, strings and tables only, no I/O. Compiled into every variant so `--help` is always complete. Single file: `src/lib.rs`. |
| `crates/fleet-config` | Configuration: `~/.config/fleet/fleet.toml` plus environment resolution, the `[domains]` enable/disable table, and state-directory paths. Single large `src/lib.rs`. |
| `crates/fleet-secrets` | The one secrets-by-name path: secrets-broker protocol first, `bws` CLI fallback, `op` CLI where a vault is named. Values are never stored, never logged, and redacted in `Debug`. Single `src/lib.rs`. |
| `crates/fleet-proxy-proto` | The session-proxy wire contract: length-prefixed JSON framing (an 8-byte big-endian length prefix plus JSON) and the hello/capability frame types. The contract lives here and nowhere else. |
| `crates/fleet-queue` | The queue domain: the repo-queue client surface plus the queue-broker and secrets-broker server roles. By far the largest crate. Key files: `src/client.rs` and its submodules under `src/client/` (`commands_lifecycle.rs`, `commands_query.rs`, `fallback_stack.rs`, `mode_report.rs`, `support.rs`, `transport.rs`, `verify_disposition.rs`), `src/fallback.rs`, `src/placement.rs`, `src/validate.rs`, `src/turso_queue.rs` and `src/turso_queue/`, `src/queue_broker.rs`, `src/secrets_broker.rs`. Standalone binaries in `src/bin/`: `repo-queue`, `queue-broker`, `secrets-broker`. |
| `crates/fleet-dispatch` | The dispatch domain: a thin client over the pm-stack `pm-dispatch` service — status, launch, and enqueue verbs. Exactly three routes: `GET /health`, `POST /v1/sessions`, `POST /v1/tasks`. Single `src/lib.rs`. |
| `crates/fleet-dispatch-test` | The dispatch-test domain: scan fleet repositories for free GitHub Actions capacity, submit a job to the Turso-backed jobs store, dispatch the pilot workflow, and track the stored status. Single `src/lib.rs`. |
| `crates/fleet-proxy` | The session proxy domain: `proxy serve` (the broker) and `proxy connect` / `proxy init` (the client). Files include `src/serve.rs`, `src/client.rs`, `src/http.rs`, `src/tunnel.rs`, `src/connect_udp.rs`, `src/files.rs`, plus extra binaries `fleet-connect-udp` and `fleet-udp-forward`. |
| `crates/fleet-laptop-agent` | The laptop-agent domain: the transport ladder (broker, Cloudflare, Tailscale, SSH) and the verbs over the laptop-agent daemon. Files: `src/lib.rs`, `src/cli.rs`, `src/client.rs`, `src/guard.rs`, `src/ladder.rs`, `src/legs.rs`. |
| `crates/fleet-search` | The search domain: the atlas-search client surface over the live Atlas service, and the `serve search` role — the Atlas service itself, rehomed from the atlas-search bin crate. Files include `src/cli.rs`, `src/client.rs`, `src/proto.rs`, `src/serve.rs`, `src/server.rs`. Feature-split into `client` and `serve`. It is the one member that consumes crates outside the workspace (`atlas-core`, `atlas-embed`, `atlas-proto` by relative path under `../atlas-search/crates/`). |
| `crates/fleet-bridge` | The bridge domain: send and receive through the message bridge's fleet mailboxes. Single `src/lib.rs` plus `src/tests.rs`. |
| `crates/fleet-agent` | The agent domain: enrol a named agent of any kind, check what a seat can reach (`doctor`), ask for the next piece of work down the intake ladder (`next`), and open a shell where builds belong. Files: `src/lib.rs`, `src/enroll.rs`, `src/kinds.rs`, `src/next.rs`, `src/shell.rs`, `src/shortlist.rs`, `src/tests.rs`. The `shell` domain is served by this crate's `run_shell` entry point. |

Where each domain lives, in one rule: a domain's client surface, parser, and usage strings live in its owning crate (or, for `secrets` and `gh`, in the binary crate); the registry entry that names it lives in `fleet-registry`; the dispatch arm that reaches it lives in `crates/fleet/src/main.rs`. See [Register a domain or capability](register-a-domain.md) for what a new domain must add in each of those places.

## Supporting files at `pkgs/fleet/`

- `default.nix` — the Nix builder for all fleet packages. See [Building fleet](building.md).
- `fleet-surfaces.json` — the placement registry: the execution surfaces placement may choose between, the owners they draw capacity from, and the laptop saturation flag. It is compiled into the binary (`include_str!`) and rendered by `fleet queue surfaces`. Add a surface by adding a registry entry and a compatible workflow.
- `migrations/` — the fleet database migrations, numbered SQL files from `0001_agent.sql` through `0026_ainventory_interpretation.sql`, including their test, check, and rollback companions.
- `repo-task-v1.schema.json` — the task packet schema that `fleet queue validate` checks a task packet against.
- `smoke.sh` — the hermetic, per-variant, post-build smoke suite. See [Testing](testing.md).
- `fleet-build-lock` — the bash helper that serialises heavy builds fleet-wide to a small number of slots. See [Building fleet](building.md).
- `rust-toolchain.toml` — the pinned toolchain. See [Building fleet](building.md).
- `README.md` — the package's own long-form document, including the operator and contributor contract (DOC-01). See [Keeping this book current](keeping-this-book-current.md).
- Helper scripts: `fleet-fetch.py` (deterministic repository snapshot fetcher writing into the fleet-queue database), `lane-claim.sh` and `lane-handoff.sh` (pilot lane claim one-shots), `enqueue-when-green.sh` (enqueue a pull request into the merge queue the moment its required check goes green), `turso-exec.py`, and the Python predecessors and peers of the Rust laptop-agent surface, `laptop-agent.py` and `laptop-agent-broker.py`.
- Subtrees that are not part of the Rust workspace: `archer-slack/` (Python Slack collector/puller), `hosted-worker/` (the TypeScript/Cloudflare Worker edition of the queue and `gh` surfaces), `placement/` (a small Go module: `policy.go`, `evaluate.go`, `cmd/placement-record/` — a proof of concept, not the live placement engine, which is the Rust code in `fleet-queue`), and `runtime-db-export/` (Python export/load tooling).

## The `doc/` versus `docs/` split

Fleet's documentation predates this book and is split three ways. Do not merge the directories; each has a distinct job.

1. `pkgs/fleet/docs/` — long-form operator documents that ship with the package. Exactly two files today:
   - `docs/verbs.md` — the canonical command reference. The repository-root `AGENTS.md` points contributors at it, and the DOC-01 contract requires it to change with every command change.
   - `docs/fleet-gh-credentials.md` — the `fleet gh` credential lifecycle.
2. `pkgs/fleet/doc/` — agent-facing runbooks inside the package. Exactly one file today: `doc/agents/remote-build-and-test.md` (dispatch builds and tests remotely; do not compile on the sandbox or the laptop).
3. Repository-root `doc/` — the fleet-wide record that lives outside the package: `doc/agents/` runbooks (including `fleet-command-channel.md` and `onboarding-an-agent.md`, which the fleet README cites by paths that look fleet-local) and `doc/decisions/`, the fleet decision records (FDR-0001 onwards, with an index in `doc/decisions/README.md`).

Two consequences to know before you go looking:

- Source files cite a design of record at `doc/2026-11-04T11-45-48Z_design_fleet_multicall_binary/`. That directory is **not** under `pkgs/fleet/doc/`; a reader who starts there will not find it.
- This book is a fourth home, and it does not replace the other three. `docs/verbs.md` remains the canonical verb reference, the FDRs remain the decision record, and this book is the narrative and procedural layer over both. The rule that keeps all of them in step is in [Keeping this book current](keeping-this-book-current.md).
