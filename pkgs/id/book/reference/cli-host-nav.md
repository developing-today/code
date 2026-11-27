# Host mode and graph navigation: `id host` and `id nav`

Source: `pkgs/id/crates/id-cli/src/cli/host.rs`, `pkgs/id/crates/id-cli/src/cli/nav.rs`, and the `Host`/`Nav` variants in `pkgs/id/crates/id-cli/src/cli/mod.rs` under https://github.com/developing-today/code/blob/main/pkgs/id/ — provenance only; this chapter is complete without following it.

Two top-level families sit outside the file/world/group families: `host`, which operates a data root of world directories, and `nav`, which walks a typed graph snapshot. The global flags from [The id command line](cli.md) apply as usual.

## `id host` — host mode over a data root

Host mode (#10693) runs and operates a **data root**: a directory holding one subdirectory per world (each with `journal.jsonl` and `modules/`, the same layout a laptop host uses) plus the host file `host.json`, which records each world's lifecycle state — **live**, **asleep**, or **archived** — and its recorded transitions. Exports are signed with `host.key` in the data root, created on first run.

The lifecycle verbs fall into three groups:

- **Read-only** — `list`, `inspect`, and `key` open no world and write nothing (except `key`, which creates `host.key` when missing).
- **Host-file editors** — `pin`, `wake-mode`, and `snapshots` act on the running host when one answers on `host.sock` in the data root; otherwise they edit the host file directly, which `id serve --world-host` and the next `id host run` pick up.
- **Running-host verbs** — `start`, `stop`, `archive`, `restore`, `export`, and `drill` reach the running host over `host.sock`, a socket only the operator who runs the host can use.

Every verb takes the data root as its first positional argument. The transition verbs take `--actor` (default `operator`), recorded on the transition they make, and most take `--json` to print the host's answer as JSON instead of a line.

### `id host run`

```
id host run <ROOT> [--start <WORLD>]... [--actor <ACTOR>]
```

Run host mode over a data root until Ctrl-C. Startup replays every world the host file records live, then starts each `--start` world. While it runs, the operator verbs above reach it over `host.sock`. Ctrl-C drains every live world and leaves the host file as it stands, so the next run replays the same worlds back to live. A data root without a host file gets one rebuilt from its directories, with every world asleep.

### `id host list`

```
id host list <ROOT> [--json]
```

List every world under a data root with its lifecycle state, storage bytes, and last state change — without opening any world and without writing the host file. This is the read-only census of a root; `id serve --world-host` refers operators to it for world states.

### `id host start` / `stop` / `archive` / `restore`

```
id host start   <ROOT> <WORLD> [--actor <ACTOR>] [--json]
id host stop    <ROOT> <WORLD> [--actor <ACTOR>] [--json]
id host archive <ROOT> <WORLD> [--actor <ACTOR>] [--json]
id host restore <ROOT> <WORLD> [--actor <ACTOR>] [--json]
```

The four lifecycle transitions, all on the running host:

- `start` — asleep → live, by replay. An archived world is refused until it is restored.
- `stop` — live → asleep: the world is drained, then recorded asleep.
- `archive` — asleep → archived: the world no longer wakes on its own, and an operator must restore it first.
- `restore` — archived → asleep.

### `id host pin`

```
id host pin <ROOT> <WORLD> [--off] [--json]
```

Pin a world always-live, or unpin it with `--off`. A serving process never closes a pinned world for idleness or to make room for another; this is also the mechanism `id serve --world-host` points to instead of an idle timeout of `0`.

### `id host wake-mode`

```
id host wake-mode <ROOT> <WORLD> <connect|operator> [--json]
```

Set how an asleep world wakes: `connect` (a client's connection wakes it — the default) or `operator` (only `id host start` wakes it).

### `id host key`

```
id host key <ROOT>
```

Print the host's export signing key (64 hex digits), creating `host.key` in the data root — readable by its owner alone — when it is missing. Give this key to whoever restores this host's exports: `id host import` pins it with `--source`.

### `id host export` / `import`

```
id host export <ROOT> <WORLD> <FILE> [--json]
id host import <ROOT> <FILE> --source <KEY> [--actor <ACTOR>] [--json]
```

- `export` — export a live world on the running host to a new file: its journal, directory facts, and active module as signed replication frames. The world keeps serving, and the file must not already exist. Start an asleep world first.
- `import` — restore an export file as a new asleep world in a data root, verifying every frame against the signing key pinned with `--source` (the exporting host's `id host key` output). Goes through the running host when one answers; otherwise writes the world directory and the host file itself. A tampered, cut, or wrongly signed file restores nothing.

### `id host drill`

```
id host drill <ROOT> <WORLD> [--json]
```

Run a one-shot restore drill for a hosted world (#13744): replay the world, export its latest signed snapshot, restore the export into a fresh scratch world outside the data root, replay the scratch to the same journal head, and compare. The source world is never mutated and the scratch is never adopted; the attempt is appended to `.drills.jsonl` in the data root. Scheduling and the missing-success alarm are follow-ups, not part of this verb.

### `id host snapshots`

```
id host snapshots <ROOT> [--world <WORLD>] [--every <DURATION> --keep <N> [--max-age <DURATION>] | --off | --inherit] [--json]
```

Show or set scheduled snapshots (#13739): how often a running host takes a signed export of each live world, and how many sets it keeps. With no `--world` this addresses the host-wide default; with one, that world's own policy and its sets on disk. `--every` (a duration such as `90m`, `6h`, or `1d`, at least 60s) and `--keep` (newest sets kept) set a policy together; `--max-age` additionally prunes older sets, and the newest set is always kept. `--off` turns scheduled snapshots off (sets already taken stay); `--inherit` drops a world's own policy so it follows the host default. With none of the setting flags, the verb only shows.

### `id host inspect`

```
id host inspect <ROOT> <WORLD> [--json]
```

Show one world's record without opening it: lifecycle state and every recorded transition, journal entries, version and torn tail, module count, storage bytes, and what the last restart replay found. Writes nothing.

Worked example (toy data):

```
$ id host list /srv/id-root
demo   asleep  1.2 MB  2026-11-20T09:14:02Z
$ id host run /srv/id-root --start demo
$ id host pin /srv/id-root demo
$ id host stop /srv/id-root demo
```

The census shows `demo` asleep; `run` brings the host up and starts it; `pin` keeps it from being closed for idleness while the host serves; `stop` drains it back to asleep.

## `id nav` — native graph navigation

```
id nav <OP> <TARGET> --graph <FILE> [--edge <KIND>]... [--direction <DIR>]
       [--cursor <CURSOR>] [--page-size <N>] [--to <NODE>] [--max-depth <N>] [--json]
```

`nav` is the CLI surface of the native graph navigation API (#7612; API v1 as landed in #15469). It walks a **graph snapshot**: a JSON file of `nodes`, `edges`, and the viewer's `hidden` ids, given with `--graph`. The arguments map one-to-one onto the wire request (`id_core::graph::navigation_wire::WireRequest`), and the CLI runs the same executor as the script/API and agent surfaces, so answers match them item for item; only the printing differs.

The five operations:

| Op          | Meaning                                            |
| ----------- | -------------------------------------------------- |
| `neighbors` | The edges at a node and the node at each far end.  |
| `backlinks` | The content that points at a node.                 |
| `threads`   | The discussion threads a node takes part in.       |
| `history`   | A node's edit states and forks.                    |
| `path`      | The shortest path to `--to`, within a depth bound. |

Flags:

| Flag          | Argument | Default          | Meaning                                                                                     |
| ------------- | -------- | ---------------- | ------------------------------------------------------------------------------------------- |
| `--graph`     | `FILE`   | — (required)     | The graph snapshot JSON file.                                                               |
| `--edge`      | `KIND`   | all kinds        | Follow only this edge kind, by record name such as `references` or `shared-as`. Repeatable. |
| `--direction` | `DIR`    | `both`           | `out`, `in`, or `both`.                                                                     |
| `--cursor`    | `CURSOR` | none             | Resume after this cursor, from the `next:` line of an earlier page.                         |
| `--page-size` | `N`      | executor default | Items per page.                                                                             |
| `--to`        | `NODE`   | none             | For `path`: the node the path should reach.                                                 |
| `--max-depth` | `N`      | none             | For `path`: the deepest walk to try.                                                        |
| `--json`      | none     | off              | Print the answer as JSON, exactly as the API returns it.                                    |

Paging is explicit: a page ends with a `next:` cursor line to pass back with `--cursor`, and a bound that cut a result is named in the output, never silent.

Worked example (toy data):

```
$ id nav neighbors NOTE --graph graph.json
$ id nav backlinks NOTE --graph graph.json --edge references
$ id nav path NOTE --to REPLY --max-depth 4 --graph graph.json --json
```
