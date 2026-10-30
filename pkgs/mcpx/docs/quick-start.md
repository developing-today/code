# Quick start

> The whole loop, in under one screen: check the machine, add one
> server, look, search, read the types, call a tool.

---

```sh
mcpx doctor                                   # runtime, config, daemon: what is missing and how to fix it
mcpx servers add fs -- npx -y @modelcontextprotocol/server-filesystem /tmp
mcpx ls                                       # namespaces, tool counts, state
mcpx search directory                         # find a tool by name or description
mcpx types fs                                 # TypeScript signatures for this one namespace
mcpx exec 'emit(await fs.list_directory({ path: "/tmp" }))'
```

That is the loop from [`AGENTS.md`](../AGENTS.md), and it is the whole
product in miniature:

1. **`ls` before anything.** Discovery is a separate, cheap step from
   use — run it whenever; it starts nothing.
2. **`search`, then `types` for one namespace only.** Signatures are
   loaded on demand, never all of them: loading everything is the
   failure mode mcpx exists to prevent. `mcpx types fs.list_directory`
   loads a single tool, cheaper still. Tool names come from the server —
   if `list_directory` is not what `types` shows, `types` is right.
3. **`exec` (or `run`) does the work.** Inside a snippet, every
   namespace is a bare identifier and results arrive unwrapped. A
   named, saved version of the same idea is a script:
   `mcpx run <name>` runs `.mcpx/scripts/<name>.ts`.

Every command above is real: verified against
`internal/cli/manual.go` (`Commands()`, the list behind `mcpx help`)
and the dispatch table in `internal/cli/dispatch.go` at the commit
this page was written. The server example is the one
`mcpx servers --help` itself uses; it needs `npx` (node). With only
deno or bun, find a first server that needs nothing extra instead:
`mcpx registry search <words>`, then `mcpx registry add <name> --write`.

Do not start with `mcpx init`: it writes a starter config whose servers
are placeholders, so the first `ls` after it shows an error row. Add a
real server first, as above.

## Make it an agent tool

The quick start above is a person at a shell. The step that makes mcpx
an *agent* tool is one of:

- give your agent [`AGENTS.md`](../AGENTS.md) — the loop, the script
  contract and the secrets rules, in the file agents already read; or
- install the opencode plugin — see [install](./install.md#channels),
  "just the plugin".

## Next

[Now what](./now-what.md) — config, surfaces, the plugin, and what to
do when something is wrong.
