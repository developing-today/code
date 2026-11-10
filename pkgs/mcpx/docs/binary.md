# The binary

<a id="binary"></a>

## Binary

mcpx ships as **one Go binary**, built from `cmd/mcpx`. This page
states what is shipped — what the binary contains, what it needs
next to it, and how the two packaged forms are built — not what
could be built. Where a fact changes per release channel, this
page says so and stops; release and install channels are a
different page's subject.

### What is in it

The version is `0.1.0`, hard-coded as the default in
`cmd/mcpx/main.go` and stamped by the package with
`-X main.version=0.1.0`; a build may override it with the same
`-ldflags` variable, and nothing else about the binary changes.

Everything else is the module's direct dependencies. `go.mod` is
the source of truth — seven direct requirements:

| module                                     | what it is for                                                                         |
| ------------------------------------------ | -------------------------------------------------------------------------------------- |
| `modernc.org/sqlite`                       | the log index: a pure-Go translation of SQLite, not a cgo binding                      |
| `github.com/charmbracelet/bubbletea`       | the full-screen browser, `mcpx tui`                                                    |
| `github.com/charmbracelet/bubbles`         | its widgets                                                                            |
| `github.com/charmbracelet/lipgloss`        | its layout                                                                             |
| `gopkg.in/yaml.v3`                         | OpenAPI documents, which are published as YAML as often as JSON                        |
| `github.com/santhosh-tekuri/jsonschema/v6` | JSON Schema validation (`internal/schemaval`)                                          |
| `github.com/tetratelabs/wazero`            | the WebAssembly runtime behind wasm tool sources and embeddings (`internal/wasmtools`) |

The three charmbracelet modules are one decision: about 1.2 MB in
the stripped binary, in exchange for not hand-writing terminal
handling. [dependencies.md](./dependencies.md) argues each of the
original five in full; it and `package.nix` still say "five"
because both predate the last two rows — trust `go.mod`, and this
table, over that count.

Two absences are as deliberate as the presences:

- **No MCP SDK.** mcpx is built on none — there is no MCP module
  in `go.mod`. The MCP client (`internal/mcpclient`), the server
  (`internal/mcpserver`) and the JSON-RPC framing are mcpx's own
  code, about a package each. That is most of why the dependency
  list stays this short.
- **Everything else is the standard library**: the daemon and its
  HTTP API, the process pools, the JSON Schema to TypeScript
  compiler, the CLI.

### How the shipped builds link

Every direct dependency is pure Go, so a `CGO_ENABLED=0` build is
fully static, and that is what the **Nix package ships**:
`env.CGO_ENABLED = "0"` in `package.nix`, stripped (`-s -w`), so
the shipped binary is a static executable that needs no libc next
to it.

The **container image** makes the other choice, on purpose for
now: its `Containerfile` defaults `CGO_ENABLED` to `1`, so the
binary it ships links glibc and the runtime stage is Debian
bookworm-slim. A static container build was measured working
end-to-end before that default was chosen — see
[container.md](./container.md), which this page links rather than
restates, because the container is still changing.

The behavioural difference between the two is narrow and worth
stating exactly: with cgo off, pure-Go `os/user` reads
`/etc/passwd` rather than asking NSS/LDAP, and the pure-Go
resolver bypasses NSS modules — on Linux only; on macOS,
`CGO_ENABLED=0` changes neither, because Go uses the system
resolver and user database there without cgo either way. Neither
difference prevents a static build, and this page never claims
they do; which form to ship where is a policy question, tracked
with the release work, not a technical block.

Size, as last measured in [container.md](./container.md): the
mcpx binary is about 14.8 MB in the container build. (The 21 MB
figure sometimes quoted is an older linux/arm64 static build;
measure the artifact you are shipping, not either number here.)

### What it needs next to it

The binary alone answers `ls`, `types`, `catalog`, `search` and
`call`. The rest of what it reaches for is external programs —
the full table, with what degrades when each is missing, is in
[dependencies.md](./dependencies.md):

- **A JavaScript runtime** — deno, bun or node — for `mcpx run`
  and `mcpx exec` only. Without one, those two commands fail and
  everything else still works.
- **`git`** — a fallback only. mcpx reads `.git` itself for the
  `repo` and `worktree` scopes (see
  [git-discovery.md](./git-discovery.md)) and runs
  `git rev-parse` only for a repository layout its own reader
  does not vouch for. Without git, those rare layouts degrade
  to keying by directory, and `mcpx doctor` says why.
- **The configured MCP servers themselves** — spawned with the
  `command`, `args`, `env` and `cwd` the config names, or reached
  over Streamable HTTP for `url` servers.

The Nix package wraps the binary with a `PATH` **suffix** naming
`git`, `deno`, `bun-bin` and `nodejs`, so those four are findable
without depending on the calling shell — and because it is a
suffix, a runtime already on your `PATH` wins over the wrapped
one. The same package installs the generated man page and shell
completions (see [the CLI page](./cli.md#cli)).

The container image ships exactly one runtime — bun — and sets
`MCPX_SCRIPT_RUNTIME=bun`, so the runtime choice inside the image
is unambiguous; it leaves git out entirely, for the reason above.
What the image deliberately does not contain, and why, is
[container.md](./container.md)'s subject.
