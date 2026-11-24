# Troubleshooting

Every entry here is a recorded failure shape, from the surveys behind this book or from a numbered disagreement in them: the symptom as an operator sees it, the cause in the code, and the fix that follows from the cause. Where the surveys record no fix, the entry says so instead of inventing one. The operator companion to this chapter is [Running a node: serve, worlds and the hosted edition](services.md); exit codes for the whole CLI are in [The id command line](../reference/cli.md).

The `id` binary's exit contract, because half of scripting against it goes wrong here: success is `0`; a handler error prints `Error: <message>` to stderr and exits `1`; a command-line parse error prints the clap error and usage to stderr and exits `2`. There is no per-command exit-code table. The REPL is the exception: once its prompt is running, a failed command prints the error and the REPL continues, still exiting `0` on a normal quit.

## An old store binary disagrees with the source (CLI-4)

**Symptom.** A command or flag behaves as no chapter in this book describes: `id serve --replace-defaults` is accepted, `--help` spells the defaults file `defaults.conf`, there is no `walk` command, or `--version` prints one line only.

**Cause.** The Nix store on a machine can hold many builds of `id-0.1.0` of different ages — the survey counted 23 — and they are not the same program. Older builds predate the current source: current source rejects `--replace-defaults` (a regression test asserts the parse error), spells the file `defaults.json`, has `walk`, and prints a two-line long version whose second line reports the embedded defaults provenance (`defaults: file sha256=… mode=append`, or the built-in equivalent). A second trap on the same machine: `/etc/profiles/per-user/user/bin/id` can be GNU coreutils `id`, a different program entirely, which answers with user and group IDs and knows none of these commands.

**Fix.** Establish which binary is running before debugging anything else: `id --version` must print the two-line form, and `id walk --help` must exist. Build or select a binary from current source and put that binary first on the path the operator actually uses. Do not debug a flag against a binary whose `--version` output does not match the source generation the book describes.

## Serve will not start: the data directory is already owned

**Symptom.** A second `id serve` in a data directory fails to open the store, or a `walk serve` reports that its node directory is held, or local commands behave as if a serve were running when the operator believes none is.

**Cause.** Two locks, both by design. The blob store is a single-opener redb database: one process holds `.iroh-store/` at a time. While a serve runs, the data directory also holds the serve lock `.iroh-serve.lock`, and the REPL reads that lock at startup to decide its mode — lock present means Remote mode, proxied to the serve. A node directory given to `id walk` is likewise held by one process at a time; `walk serve` holding it is the recorded reason another walk command against the same directory reports it held rather than waiting.

**Fix.** Find and stop the process that owns the directory, then start again. Run the second node in a second data directory (`--data-dir <DIR>` or `--new [<NAME>]`) when two nodes are wanted at once. One naming trap while checking: the lock is `.iroh-serve.lock`. A module comment in `serve.rs` spells it `.id-serve-lock`; no file with that name exists, and looking for it finds nothing and proves nothing (disagreement CLI-5). **Unverified:** the surveys record no stale-lock recovery path — no code that detects a lock left by a dead process and clears it, and no documented manual removal procedure. If a lock file remains after its process is confirmed dead, this book records the gap rather than prescribing a deletion.

## The gossip topic is not the one in `--help` (CLI-6)

**Symptom.** A node does not discover the peers it was expected to discover, or two nodes started with no topic flags do not meet, and the operator's reading of `--help` says both used the default topic `id-peer-discovery-v1`.

**Cause.** `--help` presents the hardcoded constants as the defaults: topic `id-peer-discovery-v1`, secret `id-public-discovery-v1`. The operative default normally comes first from the embedded `defaults.json` (keys `topic`, `topic_secret`, `bootstrap`, `web_bindings`, and a `mode` of `append` or `replace`). The constants are the fallback, used when the file supplies nothing or when `--no-default-topic` is passed. Two builds with different embedded defaults files therefore start on different topics while both `--help` outputs read the same.

**Fix.** Compare the defaults source, not the help text: `id --version` prints the provenance line naming the defaults file hash and mode in force. Pass `--topic <TOPIC>` (and `--topic-secret <SECRET>` where a secret is in use) explicitly on every node that must meet, or pass `--no-default-topic` on all of them to force the hardcoded constants on all of them. The same file-first rule applies to bootstrap peers (`--no-default-bootstrap` ignores the file's list) and to web bindings. Full precedence is in [Configuration and environment](../reference/configuration.md).

## `serve --web` fails: this build has no web feature

**Symptom.** Startup fails with the error stating that the web interface was requested (`--web`/`--web-listen`/`--port`/`--bind`) but the binary was built without the `web` feature, and naming the rebuild flag.

**Cause.** The web UI is the optional `id-web` crate behind the root `web` feature. The Nix package `id-web` (the default package) is built with `web + sandbox + ssh + world`; the package `id-lib` is built with `sandbox + world` only — no web, no ssh. On an `id-lib` build the serve web flags parse and then fail at runtime with that error. Before the runtime guard existed, `--web` on such a build was silently accepted and served nothing; the loud failure is the fixed behaviour.

**Fix.** Run a build with the `web` feature — the `id-web` package, or a Cargo build with `--features web`. Do not read the error as a configuration fault in the flags themselves. The same build-shape logic explains two neighbours: `--server-config <FILE>` on a build without the `world` feature fails at startup naming the missing feature, and `--world-module <FILE>` on a build without the `sandbox` feature fails the same way.

## `--web-listen` combined with `--bind` or `--port` (#2145)

**Symptom.** Serve refuses to start, naming the conflict between `--web-listen` and `--bind`/`--port`.

**Cause.** `--web-listen <IP:PORT>` replaces the whole `web_bindings` list from `defaults.json`. `--bind <IP>` and `--port <PORT>` set the single-listener form of the same thing. The two forms are alternatives, and serve reads explicitness from the command-line value sources: a `--bind` or `--port` the operator actually typed, combined with any `--web-listen`, is a startup error. Values that merely came from defaults do not trigger it.

**Fix.** Choose one form. One listener: `--bind <IP> --port <PORT>` (defaults `127.0.0.1` and `3000`). Several listeners, or a program or document listener: `--web-listen <IP:PORT>` repeated, and no `--bind` or `--port` on the same command line.

## `id group join` always fails (CLI-9)

**Symptom.** `id group join <INVITE>` parses the invitation ticket and then fails, every time, with the message that group join is unavailable: invitation parsing works, but admission and network acceptance are not implemented yet, and no group membership was recorded. The process exits `1`.

**Cause.** The handler is a stub by design at this revision. The top-level help summary for `group` lists "join" among its verbs, which is the misleading side (disagreement CLI-9); the subcommand's own help states "Unavailable" as its first line, and the handler matches it. A malformed ticket fails earlier, with the invitation parse error, which is a different failure from the stub message.

**Fix.** None in the CLI today. Do not retry, do not regenerate the ticket as a remedy for the stub message, and do not script membership on top of `group join`. Groups are created and managed with the other verbs in [Groups, documents, claims and keys](../reference/cli-groups-identity.md); admission by invitation is recorded there as unimplemented.

## World commands and `match-audit` vanish, or world bails at runtime (CLI-10)

**Symptom.** One of two shapes, on the same class of reduced build. Either `id match-audit …` and the world subcommands `dev`, `measure`, `replay` and `verify` are absent — clap answers `unrecognized subcommand` and they do not appear in `--help` — or `id world …` shows its full help tree and then every invocation fails with `this build has no world support`.

**Cause.** The feature gates are inconsistent, and both shapes are the code working as written (disagreement CLI-10). `match-audit` and those four world subcommands sit behind the `sandbox` feature in the parser itself: no `sandbox`, no variant, no help entry. The rest of `world` is not parser-gated: the command tree exists in every build, and a build without the `world` feature defers the failure to a runtime bail in the binary's dispatch. A `--no-default-features` build shows both shapes at once. The default build (`sandbox + world`) has all of them, and lacks only the web UI and world SSH serving, which are separate features. Note that `id walk` is gated by nothing and works in every build.

**Fix.** Identify the build before debugging the command: absence from `--help` means the feature was off at compile time, and no flag at run time adds it. Use the default build, or the Nix `id-web` package (`web + sandbox + ssh + world`), for the full surface. The feature table is in [The id command line](../reference/cli.md).

## REPL: `cat file | head -5` does not pipe (CLI-7)

**Symptom.** In the REPL, the line `cat myfile.txt | head -5` — copied from the README's REPL example — does not print the first five lines. It falls through to the REPL's unknown-command handling.

**Cause.** The REPL has no `|` operator. Its input language has exactly four operators: the heredoc `<<DELIM`, the here-string `<<<`, the pipe-in operator `|>` and `$(…)`/backtick command capture. A plain `|` is just another word: the line is whitespace-split, `cat` receives extra arguments `|`, `head` and `-5`, no dispatch pattern matches, and the line falls to the unknown-command handling. The README example is the wrong side (disagreement CLI-7). Errors of this kind inside the REPL print and continue; the REPL does not exit.

**Fix.** Use the operators that exist, in the direction they run. To store a shell command's output: `put $(head -5 myfile.txt) name`, or the pipe-in form `head -5 myfile.txt |> put - name`. To view part of a stored file, use the REPL's `peek <QUERY>` (5 head and 5 tail lines by default; `--lines <N>`, `--head-only` and `--tail-only` adjust it) or `cat` the whole file. The full input language is in [The interactive REPL](../reference/repl.md).

## The hosted Worker answers 503 `jurisdiction_unavailable`

**Symptom.** Run locally, the hosted Worker returns 503 with the error `jurisdiction_unavailable` for a request that is fully valid in every other respect.

**Cause.** By design. Local workerd does not implement jurisdiction restrictions, and the Worker fails closed rather than serve a request whose jurisdiction it cannot enforce. The package is a local-only scaffold: its config authorises `wrangler dev` and tests, and no production deployment. Every other `UserDO` route also runs a common guard — routing context present, runtime jurisdiction matching, and the persisted binding and jurisdiction matching the first routed request that bound them — and fails closed when any leg fails.

**Fix.** None locally, and none needed: 503 `jurisdiction_unavailable` from a local run is the correct answer, not an outage to clear. There is no flag in the package that turns jurisdiction enforcement off. What the hosted edition is and is not today is stated in [Running a node: serve, worlds and the hosted edition](services.md) and [The hosted Worker API](../reference/hosted-worker.md).

## A malformed `--server-config` fails startup, naming the parse error

**Symptom.** Serve exits `1` at startup and the error names a TOML parse problem in the server-config file.

**Cause.** The file given to `--server-config <FILE>` is loaded at startup and a malformed file fails fast, before the node serves anything. An empty file is valid; every table in the format is optional. Two documented keys are easy to miss because the module's own doc comment under-lists them: the `[budgets]` table implements five keys, including `app_inbound_bytes_per_second` and `per_peer_burst_bytes_per_minute`, where the doc comment names three (disagreement WEB-14).

**Fix.** Correct the TOML at the position the parse error names, and check any budget behaviour against the key list in [Configuration and environment](../reference/configuration.md) rather than the module doc comment. A missing feature is a different startup error with a different fix: on a build without the `world` feature, `--server-config` fails naming the feature, not the file.

## Quick checks before filing anything

| Check                                       | What it settles                                                                               |
| ------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `id --version` prints two lines             | The binary is a current-source build, and the second line names the defaults source in force. |
| `id walk --help` exists                     | The binary is not one of the old store builds (CLI-4).                                        |
| The data directory holds `.iroh-serve.lock` | A serve owns that directory now, or a lock outlived its process (unverified case, above).     |
| `id world journal check <DIR>` exits `0`    | That world's journal is valid and readable by this binary.                                    |
| Exit code is `2`                            | The command line itself was rejected; fix the flags before investigating the node.            |
