# FAQ

Answers here state only what the rest of this book establishes, and each answer names the chapter that carries the full treatment. Where the honest answer is "not yet" or "there is no such verb", the answer says that.

## What is a world?

A world is a hosted, multiplayer program space: a guest program (normally a Roc module compiled to Wasm) whose state is a fold over a signed, ordered journal of inputs, with participants joining under capabilities that say what each of them may do. One node hosts a world with `id serve --world`; everyone else's world commands are clients of that host. The concepts are in [Worlds and the Roc guest runtime](../concepts/worlds.md); joining one is [Tutorial: join a world and run a guest app](../tutorials/join-a-world.md).

## Where does my data live?

In the node's data directory: the current directory by default, the `--data-dir <DIR>` directory when that global flag is given, or `.iroh/<NAME>/` for a node created with `--new [<NAME>]`. Inside it, files live in `.iroh-store/`, the group log in `groups.jsonl`, documents and claims in `events/`, tags' registry in `.iroh-meta/registry.json`, and each hosted world's journal in `.id-worlds/<NAME>/`. The item-by-item table is in [Backup, restore and data directories](backup-and-restore.md).

## What is the difference between a tag and a claim?

A tag is file metadata: a key and an optional value attached to a stored file, held in the tag store, set with `id tag set <FILE> <KEY> [VALUE]`. A claim is a named statement about a subject, held in the event store, set with `id claim set <SUBJECT> <NAME> [VALUE]`, and it carries a scope — `shared` or `local` — that a tag does not have. They are different stores, different commands and different subjects: tags describe files; claims make scoped statements about subjects. See [The tag family](../reference/cli-tags.md), [Groups, documents, claims and keys](../reference/cli-groups-identity.md), and the worked example in [Tutorial: organise a library with tags and claims](../tutorials/organise-with-tags.md).

## Can I delete a file?

Yes — but not from the top-level command line. The clap CLI has no delete, rename or copy command at all. Those operations exist in the interactive REPL (`delete <NAME>` / `rm <NAME>`, `rename <FROM> <TO>`, `copy <FROM> <TO>`), in the web UI, and in the shared FileOps layer underneath both. Underneath, `FileOps::delete` is a hard delete of the name, its metadata and its archives; the blob bytes themselves survive until garbage collection, and the web layer builds a separate soft-delete and restore pair on top. See [The interactive REPL](../reference/repl.md) and [Sync, replication and FileOps](../concepts/sync.md).

## Why is there no MCP server yet?

Because the code contains a translator and a policy point, not a server. What exists is a library in `id-caps`: `mcp_bridge` translates one MCP tool call into a world-session frame (or a locally rendered answer, or a session close), and `mcp_policy` checks the capability ceiling, charges quota before execution, and writes exactly one signed audit event per action. There is no standalone MCP server binary, no stdio or HTTP MCP transport, and no JSON-RPC `tools/list` implementation in `pkgs/id`, and no ID entry is wired into the mcpx configuration. The 8 v1 tools exist as definitions for an agent already inside a world session. The full account is in [The MCP bridge](../reference/mcp.md) and [Agents in worlds: the MCP bridge and policy](../concepts/mcp-and-agents.md).

## What does the hosted edition run today?

A local-only scaffold. The `pkgs/id/hosted-worker` package runs under `wrangler dev` and its test harness; its config authorises no production deployment, and locally it fails closed with 503 `jurisdiction_unavailable` for every fully valid request, by design, because local workerd does not implement jurisdiction restrictions. There is no hosted URL, no deployed per-user service, and no operator action that turns the scaffold into one. See [Running a node: serve, worlds and the hosted edition](services.md) and [The hosted edition](../concepts/hosted.md).

## How do two nodes find each other?

Through the discovery layers serve runs by default: gossip on the topic from `defaults.json` (fallback constants `id-peer-discovery-v1` and `id-public-discovery-v1`), mDNS on the local network, bootstrap node IDs, and relay-assisted connection. Each layer has an off switch — `--no-gossip`, `--no-mdns`, `--no-default-bootstrap`, `--no-relay` — and the mainline DHT is opt-in with `--dht`. The operational table is in [Running a node: serve, worlds and the hosted edition](services.md). One warning belongs here: `--help` calls the fallback constants "the default"; the file comes first. That trap is in [Troubleshooting](troubleshooting.md).

## I ran `id` and got user and group IDs, not a prompt. What happened?

That binary is GNU coreutils `id`, a different program that happens to share the name; on the surveyed machine it sits at `/etc/profiles/per-user/user/bin/id`. This book's `id`, run with no subcommand, starts the interactive REPL. Check which binary the path resolves, and check `id --version`: this book's binary prints a two-line long version whose second line reports the embedded defaults provenance. Old builds of this book's own binary are a second version of the same trap; see [Troubleshooting](troubleshooting.md).

## Is there a backup command?

No whole-node backup or restore verb exists. The narrow exceptions are `id world journal backup <DIR> <OUT>` for one world's journal (the output path must not already exist, and the journal must pass `journal check` first) and the `id key export` family for keys. Everything else is the operator copying the data directory with serve stopped, as laid out — and marked unverified as a product procedure — in [Backup, restore and data directories](backup-and-restore.md).

## Can anyone write to my node?

Only writers the node admits: this machine's own keys, node IDs passed with `--allow-node <IDS>`, and node IDs listed one per line in the `.iroh-allowed` file in the data directory. The exception is `--open-writes`, which lets any peer write and is documented in its own help text as insecure. See [Running a node: serve, worlds and the hosted edition](services.md).

## Why did `id world --help` work on a build where every world command fails?

Because the world command tree is not gated in the parser, while its dispatch is gated on the `world` feature: help prints the full tree in every build, and a build without the feature fails each invocation at runtime with `this build has no world support`. Neighbouring commands gate the other way — `match-audit` and four world subcommands disappear from the parser entirely without the `sandbox` feature. Both shapes, and the build that shows each, are in [Troubleshooting](troubleshooting.md).

## What is a guest?

The program that runs inside a world: a Roc (or Rust SDK) module compiled to Wasm, exporting the Plaza ABI, which receives inputs through the host, asks for effects through capabilities, and returns views. The host owns the journal, the capabilities and the limits; the guest owns the world's logic and its folded state. See [Worlds and the Roc guest runtime](../concepts/worlds.md) and [Glossary](glossary.md).
