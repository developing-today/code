# Finding commands, frames, capability providers and hosted routes

*Research snapshot of November 23, 2026. Code and issue observations refer to the cited revisions and collection times.*

The [World capability catalogue](research-catalogue.md) adds version, signature source, bounds, failures, purity, fuel treatment and host evidence to the earlier identifier inventory.

The source ledger identifies **302 distinct named surfaces** in commit `4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a`. Each CSV row has a path and line, a commit-pinned source URL, its extraction method, one proposed owning chapter, and a deeper question that remains. This is a static inventory. These counts describe the source tree at the given revision and do not establish test or deployment results. [Surface ledger](evidence/surface-coverage.csv) · [Root source](https://github.com/developing-today/code/tree/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id)

| Source scope | Rows | What was counted |
|---|---:|---|
| CLI | 126 | Every canonical variant of all 14 Clap Subcommand enums in id-cli's cli directory. The count includes 32 top-level variants and nested command groups. Aliases are properties of the canonical command. |
| Cargo workspace | 17 | 16 member manifests under crates/ and the root id package. Standalone examples and vendored crates are excluded. |
| Capability registry | 36 | 34 host catalogue entries and two declared client providers. The test probe is outside production. The catalogue includes six pure source modules. |
| Shell frames | 78 | **43** client and **35** server names in id-shell v1 revision 15. A name allowed in both directions has two contracts. |
| Local Worker dispatch | 15 | Ten literal method/path branches and five method/parameterized-path branches in UserDO. |
| Currency gateway | 6 | Six method/path alternatives in the separate single-process gateway PoC. No route in this group is advertised as a live Worker endpoint. |
| Roc app sources | 11 | One source app under apps/ plus ten example sources under examples/ with an app [program] header. Two of those ten are deliberately failing compile fixtures. This answers how many *Roc program sources* exist under those paths; it is a different measurement from counting example directories or shipped artifacts. |
| Roc guest entrypoints | 8 | Required source-facing fields, extracted from the ID Roc platform's requires block. |
| Role entrypoints | 2 | Default REPL dispatch and the public Rust facade, with explicit entrypoint evidence. |
| Shell transports | 3 | Iroh, WebSocket and SSH exec as contracts, at the same revision as the frames. |

## Locate the appropriate interface

| Task | Verified entry point | Meaning and scope |
|---|---|---|
| Start an interactive local session | `id` or `id repl` | Empty command enters the local REPL. The REPL has additional interactive syntax. |
| Serve content or a world | `id serve` | The operator starts the serving process. `id publications` lists dynamic publications on the running process. `id world join` connects to a world host. |
| Inspect the portable shell interface | `id world contract` | Prints the contract. `--check FILE` checks a transcript. `--check-suite FILE` checks a suite, with `pkgs/id/contract/shell-suite.v1.json` as the in-tree manifest. |
| Inspect a durable world | `id world journal check DIR --json` or `id world provenance DIR --json` | Read a local world's journal metadata. `id world owner DIR --json` reports on-disk facts and can mark live facts unavailable. |
| Read a world's published records | `id world records NODE --prefix PREFIX` | Requires world read authority. The parser accepts the world capability from `ID_WORLD_CAPABILITY`. World selectors and optional direct addresses live in the command's own argument structure. |
| Join or resume a world | `id world join NODE --after SEQUENCE` | Resumes from a sequence where supported using an authorized world capability. `--command-mode` interprets local slash commands. `--json` applies to those responses. |
| Explore federation | `id walk init DIR --name NAME`, `id walk reports DIR`, `id walk read DIR ADDRESS` | A stored walk node provides a reproducible, small federation example. `id walk serve DIR` offers a peer token and `id walk sync DIR --peer PEER` consumes it. The `write` command uses named demo writers. |
| Explore graph and identity state | `id doc frontier`, `id claim review`, `id writer-sets show`, `id keyset recovery status` | These are command names. Each group has its own arguments and connection rules. The ledger links directly to each definition. |
| Work on guest artifacts | `id world compile`, `id world measure`, `id world replay`, `id world verify` | Measure, replay and verify are conditionally compiled with sandbox support. Use the relevant command's own argument declaration before constructing a job. |
| Validate a demo match | `id match-audit export-tic-tac-toe`, `id match-audit verify` | The feature-gated group identifies signed v0 match-log tooling. Its existence does not establish full acceptance of the attestation/audit epic. |

The commands above are references. They were not run against the laptop or a remote service during this survey. The crosswalk gives each command its registered spelling, including explicit `id group role` and `id tag del`, with declared aliases in the evidence column. [CLI definitions](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-cli/src/cli/mod.rs#L623) · [World arguments](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-cli/src/cli/world.rs) · [Walk arguments](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-cli/src/cli/walk.rs)

### Worker and native import

| Route family | Source-dispatched methods | Contract boundary |
|---|---|---|
| User object metadata | GET /v1/state, /v1/usage, /v1/export | The export here contains KV values and blob inventory. This route is distinct from bridge portability export. |
| Signed bridge | GET /v1/bridge/challenge, /heads, /export, /ws and POST /v1/bridge/push, /pull (all under /v1/bridge) | Resolved binding and jurisdiction route to the per-user object. HTTPS uses per-call proof on protected legs. WebSocket upgrade performs its own authentication exchange. |
| Blob data | PUT /v1/blobs, GET and DELETE /v1/blobs/{hash} | The hash path requires 64 lowercase hex digits. Data and metadata are mediated by the per-user object. |
| KV data | PUT, GET and DELETE /v1/kv/{key} | The path matcher limits the key to 1–128 supported ASCII characters. |
| Stub currency gateway | POST /v1/transfers, /v1/holds, /v1/holds/{id}/post or /void. GET /v1/balances/{account} or /v1/transfers/{id}. | Transfer/hold intent, settlement choice and reconciliation belong to the separate currency gateway implementation. Its API has no inferred public deployment. |

The checked-in Wrangler configuration identifies itself as a local scaffold. A GET of `/v1/bridge/export` on a live account was outside this source review. The native importer takes verified data through normal object/blob ingest and returns individual outcomes. The surveyed Worker has no `POST /v1/import` branch. Use the native import seam when describing that half. [Worker configuration](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/hosted-worker/wrangler.toml) · [UserDO branches](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/hosted-worker/src/user-do.ts#L384) · [Currency gateway](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/hosted-worker/src/currency/gateway.ts#L184) · [Native import](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/src/hosted_ingest.rs)

## Where the chapters need more than a name

| Owning chapter | Assigned ledger rows | Needed depth |
|---|---:|---|
| Charter | 2 | Show process, user role, signed object and projection boundaries. Map local-first intent to a reproducible disconnected example. |
| Crates | 17 | Compare source references, feature edges and root exports. Document API stability for libraries embedded by external applications. |
| Roc | 67 | Connect all 36 registry rows to pure modules, hosted executors, costs, refusal branches and platform proofs. Track negative compile tests separately. |
| Clients | 86 | Describe frame/phase state machines and validated view/action mappings. Confirm semantic, accessibility and physical-device behavior. |
| Graph | 24 | Distinguish graph record publication, visibility, local claims, writer-set discovery and Atlas indexes. Demonstrate bounded path/cursor behavior. |
| Federation | 27 | Separate shell transport, object exchange and hosted routing. Verify each mode against an admitted durable outcome after disconnect/rejoin. |
| Lifecycle | 52 | Detail install, signing/rotation/recovery, admission, artifact versions, journal restoration and revocation with replay cases. |
| Tokens | 6 | Connect PoC routes to the existing asset type, accounting, double-spend rules, fuel admission and production decision boundaries. |
| Store | 21 | Show failure semantics for retention, durable state, export/import, deduplication and quota. Cover the reserved IndexedDB fixture decision. |

An app/feature count from issue labels, a complete inventory of Rust public types, all argument flags and environmental combinations, non-Clap REPL-only commands, web UI routes outside hosted-worker, dynamic mounts, runtime availability and a physical-device test matrix are outside this ledger's extraction domain. Record each one as its own inventory before claiming platform-wide completeness.

## Reproducing the ledger

Use a checkout or archive at the commit above as the first argument to `python3 extract-surface-coverage.py REPOSITORY_ROOT OUTPUT_DIRECTORY`. The extraction script uses Python's standard library. It identifies Clap `Subcommand` enums, checks the known parent mapping against the set of declarations, checks that every nested group has its canonical parent, and extracts explicit names/aliases and local feature gates. It reads workspace and frame schemas as TOML/JSON, scans the normal/declared client manifests, identifies both literal and patterned Worker routes, and explicitly expands the PoC gateway's two union branches. Each pair of surface type and name must be unique. This parser is specific to the reviewed source conventions; a renamed enum, changed attribute form, generated command or router change requires reviewing its extraction coverage. For a release proof, compare the static names with deployed feature-specific help and handler tests through Fleet and require each row to link to a stable chapter anchor before release. [Generator](evidence/extract-surface-coverage.py)
