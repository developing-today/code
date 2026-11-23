# The shape of the system

This chapter is the whole system on one page: the crates, the surfaces,
how a file moves, how a world runs, and where data lives. Every other
part of this book expands one section of this page.

## The workspace: a facade plus 16 crates

`pkgs/id/` is a Rust workspace of 17 packages: the root package `id`
version `0.1.0` (the facade) and 16 crates under `pkgs/id/crates/`.
Default members are 15 packages: the root plus 14 crates. The two shell
crates, `id-native-shell` and `id-hw-shell`, are workspace members but
not default members, so a plain `cargo build` in `pkgs/id/` never
acquires the Bevy desktop or hardware-shell dependencies. Two vendored
crates, `vendor/p2panda-sync` and `vendor/p2panda-encryption`, are
excluded from the workspace.

The facade is not a re-export shim. `pkgs/id/src/` holds 26 modules of
first-party code (among them `backup`, `currency`, `fileops`,
`grant_service`, `hosted_binding`, `hosted_export`, `hosted_ingest`, and
`structured_logging`) alongside the re-exports that make the split
crates reachable as `id::<name>`. The shipped `id` binary is itself an
embedding consumer of that surface: `pkgs/id/src/main.rs` imports the
CLI types and command functions through `id::…`.

Dependency direction runs one way, from leaves to surfaces:

| Layer             | Crates                                                               | What each owns                                                                                                                                                                                                                                                                                                                                         |
| ----------------- | -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Foundation        | `id-core`                                                            | The leaf modules everything depends on (10 production dependents): content IDs, the graph record model, signed envelopes, discovery, the port contract vocabulary, and the constants `KEY_FILE = ".iroh-key"`, `CLIENT_KEY_FILE = ".iroh-key-client"`, `SERVE_LOCK = ".iroh-serve.lock"`                                                               |
| Data              | `id-store`, `id-events`                                              | `id-store`: the blob store, tags, documents, search, and the signed event-header chain. `id-events`: the event store, the group projection, writer sets, and sync gating                                                                                                                                                                               |
| Platform services | `id-caps`, `id-groups`, `id-sync`, `id-directory`                    | `id-caps`: the capability catalog, execution limits, and the world manifest. `id-groups`: group policy and enforcement, the blob gate, group encryption. `id-sync`: log sync, the meta protocol, FileOps, envelopes, and artifacts. `id-directory`: the home-server directory, invites, keysets, recovery, and seitan admission                        |
| Worlds and web    | `id-worlds`, `id-web`                                                | `id-worlds`: the world runtime, sessions, the wasmtime sandbox, and cross-world authority (75 `.rs` files, the largest crate). `id-web`: the HTTP and WebSocket server, routes, templates, and markdown                                                                                                                                                |
| Application       | `id-cli`                                                             | Argument parsing, command implementations, the REPL, and the server-config loader                                                                                                                                                                                                                                                                      |
| Standalone        | `id-config`, `id-currency-registry`, `id-render`, `id-import-notion` | `id-config`: the versioned configuration contract (no production dependents; the facade names it only in dev-dependencies). `id-currency-registry`: the currency control-plane registry (facade only). `id-render`: the rendering pipeline compiler core (no dependents). `id-import-notion`: the Notion importer inventory primitives (no dependents) |
| Shells            | `id-native-shell`, `id-hw-shell`                                     | The Bevy native reference shell and the ESP32-class hardware shell spike. Both consume the facade with `default-features = false, features = ["world"]`                                                                                                                                                                                                |

The split is recorded history, not a plan: the code was one crate and
was divided in numbered steps of umbrella issue #3447 — `id-core` in
Phase 1, then `id-caps`, `id-store`, `id-events`, `id-sync`, `id-groups`,
`id-worlds`, `id-directory`, `id-web`, and `id-cli` in steps 1 to 7b.

## The surfaces

Two binaries exist in the workspace, both by Cargo's implicit
`src/main.rs` rule; no `[[bin]]` section exists in any workspace
`Cargo.toml`.

- **The CLI** (`id`, from the root package). 32 top-level commands and
  113 leaf commands with 188 distinct long flags (surveyed 2026-11-23).
  `id serve` is the only command that hosts: blobs, optionally a world,
  and optionally the web UI. Every other command is a client or a local
  operation. Running `id` with no subcommand starts the REPL.
- **The REPL** (`id repl`, alias `id shell`, or bare `id`). The REPL has
  its own parser — a line is whitespace-split and matched against literal
  patterns — so clap flags and defaults do not exist inside it except
  where a hand parser re-implements them. It adds commands the CLI does
  not have (`delete`, `rename`, `copy`) and lacks the CLI's `serve`,
  `world`, `group`, `doc`, `claim`, `key`, and `keyset` families. Its
  modes are `Local` (direct store), local-serve (operations proxied to a
  running local `serve`, detected by `.iroh-serve.lock`), and
  `RemoteNode` (started as `id repl <NODE>`).
- **The web UI** (`id-web`, behind the root crate's `web` feature).
  Served by `id serve --web` with bind default `127.0.0.1` and port
  default `3000`. The main router holds 54 distinct paths and 58 method
  bindings (surveyed 2026-11-23): file pages, a ProseMirror collaborative
  editor, the world bridge surface, the program gallery, and three
  WebSocket protocols (collaborative editing, tag changes, and the world
  session bridge). A second, program-only router holds 10 paths and is
  what a publication mount serves.
- **The hosted Worker scaffold** (`pkgs/id/hosted-worker`). A Cloudflare
  Worker front door forwards already-authenticated requests to one
  `UserDO` Durable Object per bound user; the object exposes 15 method
  routes over key-value data, blob references, usage, export, and the
  signed-object bridge. It is a local-only scaffold: its `wrangler.toml`
  authorizes no account-level deployment, and under local `workerd` it
  fails closed with 503 `jurisdiction_unavailable` for every fully valid
  request, by design. A second package, `pkgs/id/worker-poc`, is a proof
  of concept that runs one example world in a Worker isolate; it is not
  a production path.
- **The native shell** (`id-native-shell`, Bevy). A desktop client for
  the F13.1 shell contract in `pkgs/id/contract/shell-contract.v1.json`
  (contract `id-shell`, version 1, revision 15). It connects to a world
  host, renders the host's view frames, and adds no frame kinds. Its
  argument parser is hand-rolled: it has no `--help` or `--version`, and
  an unknown flag is an error with exit code 2.
- **The hardware shell** (`id-hw-shell`). A library crate, not a binary:
  the shell contract reduced to presence, journal-head state, and
  confirm/act form answers, for ESP32-class devices. It is
  transport-free by design and builds exactly two client frame kinds,
  `join` and `form_result`.

Agents reach worlds through one more surface, the MCP bridge: a library
translator in `id-caps` (8 pinned MCP v1 tools) plus its policy point.
There is no MCP server binary; see [The MCP bridge](../reference/mcp.md).

## How a file moves

1. **Store.** `id put hello.txt` hands the bytes to FileOps
   (`id-sync`), the single put/rename/copy/delete implementation used by
   the CLI, the REPL, the meta protocol, and the web UI. The bytes land
   in the Iroh blob store as an immutable blob named by its hash. A name
   in the tag store points at the hash, and FileOps stamps the identity
   tag keys `name`, `file`, and `path`, plus the `origin` key at first
   write.
2. **Rename and replace.** Renaming a name does not move bytes; it moves
   the name and archives the displaced name as
   `{name}.archive.{unix-ts}`. That archive name form is a reserved
   namespace: FileOps refuses it as a user file name.
3. **Fetch.** `id get hello.txt` resolves name to hash through the tag
   store and reads the blob. With a 64-hex node ID as the first
   positional argument, the same commands operate on a remote node
   instead: the request travels over the meta protocol (ALPN
   `/iroh-meta/3`) to that node's store.
4. **Serve and sync.** `id serve` holds the store open for peers,
   announces through gossip discovery (topic `id-peer-discovery-v1`),
   mDNS local discovery (on by default), and DHT lookup (off by
   default; `serve --dht` enables it). `id send` copies a file from one
   server to another with no bytes passing through the client. Signed
   events — documents, claims, group operations — move through the
   event store and log sync, not through the blob path.

## How a world runs

1. **Author.** A world program is Roc source against the shared platform
   package in `pkgs/id/examples/roc-world/`. `id world new <DIR>`
   scaffolds `main.roc`, `build.sh`, and `README.md`. The guest provides
   eight functions: `init`, `update`, `view`, `layout`, `records`,
   `wants`, `snapshot`, and `restore`.
2. **Compile.** The program compiles to an import-free Wasm module under
   Plaza ABI v1: the module exports `memory` plus 14 `plaza_*` exports
   and imports nothing — no WASI, no filesystem, no network, no clock,
   no randomness. Nine Roc guest apps ship in the tree (surveyed
   2026-11-23), including the packaged two-player tic-tac-toe in
   `pkgs/id/apps/tic-tac-toe/`.
3. **Host.** Only `id serve --world` hosts a world. Hosting requires
   `--world-admin-token`; the world name defaults to `lobby` and its
   files live in `.id-worlds/<NAME>/`. The host runs the guest in the
   `id-worlds` wasmtime sandbox under literal limits: fuel default
   `10000000`, guest memory default `16` MiB, message size default
   `1024` KiB, total world memory default `64` MiB, open worlds default
   `256`, sessions default `1024`, checkpoint interval default `1000`
   events.
4. **Effects by capability.** The guest returns its effect requests from
   `wants` as a JSON document of subscriptions and capability requests
   (`time.now`, `chat.say`, `records.read`, `kv.get`, and the rest of
   the catalog in `id_caps::world_caps`). The host answers each request
   with a participant-0 update event, journalled so replay feeds the
   same events back. Grants and ceilings are enforced on the host: the
   default capability policy is `deny`, and an ungranted request returns
   a refusal event with a stable error code such as `cap_denied`.
5. **Join.** Every `id world …` subcommand is a client. A participant
   joins with a capability from `id world invite`, which requires the
   admin token and a display name; roles run from `spectator` to
   `builder` and `moderator`, invite expiry defaults to 7 days, and the
   maximum expiry is 30 days. Shells — terminal, browser, native,
   hardware — speak the same session protocol to the host over Iroh,
   WebSocket, or SSH.

## Where data lives

| Data                                                   | Store                                                                                                 | Location                                                                                                              |
| ------------------------------------------------------ | ----------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| File bytes                                             | Iroh blobs `FsStore` (redb plus data files)                                                           | `.iroh-store/` in the data directory; in-memory under `serve --ephemeral`                                             |
| File names and metadata tags                           | Tag store over Iroh docs namespace pairs (global, node-private, custom)                               | Under `.iroh-store/docs/`; namespace registry at `.iroh-meta/registry.json`, registry version 1                       |
| Signed events (documents, claims, groups, writer sets) | Event store, redb file `events.redb`, format marker `id.events.store.v9`                              | The `events/` directory in the data directory (the `--event-store` default)                                           |
| Group log                                              | Append-only JSONL                                                                                     | `groups.jsonl` in the data directory (the `--groups-log` default)                                                     |
| Directory accounts, groups, invites                    | Directory journal, one JSON entry per line, 17 entry kinds                                            | `directory.jsonl` beside the server config that provisions it                                                         |
| World state                                            | Per-world journal plus content-addressed Wasm modules                                                 | `.id-worlds/<NAME>/` in the data directory                                                                            |
| Node and client keys                                   | Key files                                                                                             | `.iroh-key` and `.iroh-key-client` in the data directory; keysets in `.id-keyset.jsonl` and `.id-keyset-client.jsonl` |
| Hosted user data                                       | One `UserDO` Durable Object per bound user (SQLite schema version 3) plus an R2 bucket for blob bytes | In the hosted Worker scaffold only; no deployment exists                                                              |

The full inventory is [Data stores and schemas](../reference/data-stores.md).

## Shipped and design: the discipline of this book

ID has two records, and this book keeps them separate. The shipped
record is the source tree: the surveys behind this book read the parser,
the crates, the routers, and the Worker at commit `1ccb4a19457` on
2026-11-23, and the getting-started, concepts, reference, and library
parts describe only what that tree does. The design record is the
archive: 408 ID design records among the 2,917 paths under the
repository's `doc/` directory (counted 2026-11-23), plus the open issues
they name. The design part of this book describes that record as design.

The two records disagree in named places, and the disagreements are data,
not embarrassments to smooth over. The surveys recorded 56 of them: the
README gives `list` an `ls` alias the parser does not have (CLI-1); the
README's Cargo quick start misdescribes `--no-default-features` (CLI-2);
the hosted Worker README's route list describes a first slice the code
has passed (WEB-9); the MCP v1 tool list contains `presence.set`, a tool
the bridge refuses on every call (WEB-13); `README.original.md` is not an
original README at all but the earliest ID concept sketch, with an access
model the capability system replaced (DES-8). Where a chapter meets one,
it states the source behaviour, names the disagreement by its survey
number, and moves on. A claim this book could not pin against the source
is marked unverified in the chapter where it appears; a claim with no
such mark is one the surveys verified.
