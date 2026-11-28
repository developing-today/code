# Glossary

One line per term, in plain words. Each definition states what the book's sources establish; the chapter that treats the term in full is named where one chapter owns it.

- **archive tag** — the reserved name form `<name>.archive.<unix-ts>` given to a file's previous content when it is renamed or replaced; the writer refuses that form as a user file name.
- **bootstrap** — the starting node IDs a node contacts to enter the gossip network, taken from `--bootstrap <IDS>` or from `defaults.json` unless `--no-default-bootstrap` is given.
- **capability** — an unforgeable grant, presented as a token, that lets its holder perform named actions in a world or against a store; world clients present one with `--capability <C>` or `ID_WORLD_CAPABILITY`.
- **ceiling** — the upper bound on what a holder may do: a set of named capabilities (for an agent, also its worlds, object classes and program allowlist) that grants can narrow and never widen.
- **checkpoint** — a saved fold of a world's journal at one point, written every `--world-checkpoint-every <N>` events (default `1000`), so replay can start there instead of at event one.
- **claim** — a named statement about a subject, held in the event store with a scope of `shared` or `local`, set with `id claim set`. Distinct from a tag.
- **data directory** — the directory a node keeps all its state in: the working directory by default, `--data-dir <DIR>` when given, or `.iroh/<NAME>/` under `--new [<NAME>]`.
- **defaults.json** — the defaults file embedded in the binary at compile time, supplying the gossip topic, topic secret, bootstrap IDs and web bindings before command-line flags override them.
- **delegation** — the act, and the resulting record, of granting part of one's own authority in a group to another account, done with `id delegate <GROUP> <GRANTEE>` and undone with `id revoke`.
- **directory** — the account and group register: who has signed up, their keys, tiers, groups and friendships, recorded as entries in `directory.jsonl` and served to clients through the directory routes.
- **FileOps** — the single implementation of put, rename, copy and delete for a stored file, shared by the CLI, the REPL, the meta protocol and the web UI (`crates/id-sync/src/fileops.rs`).
- **fixture** — a recorded set of world inputs (with a module and a seed) used to replay a world deterministically in tests and in `id world replay` and `id world verify`.
- **gallery** — the program registry a node can serve from `--gallery-dir <DIR>`: signed `.idpb` bundles, collections, and a public transparency list of removals, refusals and reinstatements.
- **group** — a named set of accounts with roles, created and managed in the group log (`groups.jsonl` by default) and governed by a group policy.
- **guest** — the program inside a world: a Roc or Rust module compiled to Wasm that exports the Plaza ABI and owns the world's logic and folded state.
- **journal** — a world's signed, ordered record of inputs; the world's record, from which its state is folded and replayed.
- **node ID** — a node's public identity, written as 64 hex characters; commands that take a node fail with `invalid node ID: must be 64 hex characters` on anything else.
- **Plaza ABI** — the host/guest interface a world module exports (Plaza ABI v1): `memory` plus the `plaza_*` exports — `plaza_init`, `plaza_update`, `plaza_view`, `plaza_snapshot` and kin — through which the host runs the guest.
- **provider** — a host-side service a guest can ask for through the capability protocol (an "effect"), whose offer and reply pass through the world session; a provider declaration moves through the lifecycle Filed, Admitted, Live, Suspended or Tombstoned.
- **RegistryEntry** — one entry in a signed registry list: a world announcement kept verbatim, the time it was included, and its registry sequence number.
- **replay** — re-running a world's journal through its module from the start, or from a checkpoint, to rebuild or verify the folded state; `id world replay` compares the result against a fixture.
- **serve lock** — the file `.iroh-serve.lock` in the data directory while a serve process owns it; the REPL reads it to decide whether to proxy operations to that serve.
- **shell contract** — the fixed set of frames and behaviours every client shell (terminal, web, native, hardware) must implement to talk to a world, checked by `id world contract`.
- **tag** — a key and optional value attached to a stored file as metadata, held in the tag store, set with `id tag set <FILE> <KEY> [VALUE]`. Distinct from a claim.
- **tombstone** — the registry record that delists a world: its world ID, the removal time, a reason of at most 200 characters, and its registry sequence number.
- **topic** — the gossip channel name nodes meet on (with its topic secret); the shipped fallback values are `id-peer-discovery-v1` and `id-public-discovery-v1`, behind `defaults.json`.
- **UserDO** — the per-user Durable Object class in the hosted Worker, one object per binding ID, holding that user's key-value data, blob references and bridge store at SQLite schema version `3`.
- **world** — a hosted, multiplayer program space: one guest program, one journal, and participants joining under capabilities; hosted by `id serve --world` and stored under `.id-worlds/<NAME>/`.
- **writer set** — the set of writers authorised for an event or document chain at a given point, inspected and expanded with the `id writer-sets` commands.
