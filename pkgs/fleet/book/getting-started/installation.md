# Installation and bootstrap

There are two ways to get fleet onto a machine:

1. **The static release** — a single statically linked musl binary published
   as a GitHub release asset. This is what a fresh sandbox seat installs,
   through `scripts/muse/bootstrap.sh`. It needs no Nix and no shared
   libraries, which is the point: a glibc-dynamic build names Nix store paths
   that a non-Nix host does not have.
2. **A Nix build** — one of the variant packages from the repo flake, for
   machines that are part of the Nix fleet (the laptop, NixOS hosts).

## The static musl release

The release is produced by `scripts/muse/publish-fleet.sh`:

- It builds the `fleet-static` package (the full capability profile on the
  musl `pkgsStatic` Rust platform) in a throwaway detached git worktree at the
  requested revision (default `origin/main`).
- It refuses to publish an artifact that is not statically linked, and it
  probes the artifact's `capabilities` output: a build whose `bridge` domain
  is not available is fatal, because a Muse seat's bootstrap requires the
  bridge-capable full profile.
- The release tag is `fleet-static-<short>`, where `<short>` is the first 7
  characters of the commit SHA. The asset name is
  `fleet-x86_64-linux-musl`, published alongside
  `fleet-x86_64-linux-musl.sha256` containing the asset's SHA-256 sum.
- The script prints exactly two lines for the operator to pin in the
  bootstrap script: `tag=fleet-static-<short>` and `sha256=<sum>`.
  Re-publishing an existing tag only re-uploads when the sum differs.

Source: https://github.com/developing-today/code/blob/main/scripts/muse/publish-fleet.sh

## Bootstrap: `scripts/muse/bootstrap.sh`

A Muse sandbox seat is rebuilt from scratch whenever the sandbox is recycled:
no fleet binary, no laptop link. `bootstrap.sh` rebuilds the seat. Every phase
is idempotent and skips work already done. Exit codes: 0 success, 1 a phase
failed (the message names it), 2 usage. Phases are selectable with
`--only <list>` / `--skip <list>`; `--all` additionally restores `~/git` and
`~/agents`.

**The pin.** Read the matched asset URL and SHA-256 from the same checkout of
`scripts/muse/bootstrap.sh` that you intend to run:

```sh
grep -E '^FLEET_(URL|SHA256)=' scripts/muse/bootstrap.sh
```

Bootstrap uses those defaults and verifies the downloaded binary. When
overriding the release, supply both `FLEET_URL` and `FLEET_SHA256` for the
same published asset. After installation, check `fleet version` and
`fleet capabilities` on the seat.

`FLEET_AGENT` (default `muse`) and `FLEET_AGENT_GROUPS`
select the seat identity; groups are never hardcoded for another agent kind.

**The phases**, in the order `PHASES="tools token restore tailnet fleet persist init check"`:

1. **tools** — downloads the pinned fleet binary, verifying its SHA-256, into
   `$HOME/.local/share/muse-bootstrap/fleet/fleet`. It is kept under the name
   `fleet` in its own off-PATH directory because fleet picks its verb set from
   argv0 (the name it is invoked under). The download is skipped when the
   existing copy is present and passes the bridge-capability probe. The phase
   also installs `bws`, `restic`, and `tailscale` into the seat's bin
   directory; a tailscale failure only skips the tailnet phase.
2. **token** — obtains the Bitwarden Secrets Manager access token for the
   seat.
3. **restore** — restores the seat's files with restic, and provisions the
   three shared broker token files from Bitwarden Secrets Manager, each with
   owner-only (0600) permissions:
   `fleet.secrets-broker-token` → `~/tools/secrets-broker-token`,
   `fleet.queue-broker-token` → `~/tools/queue-broker-token`, and
   `fleet.laptop-agent-token` → `~/tools/laptop-agent-token`. Bitwarden is the
   source of truth for these three names. A missing name is not fatal: the
   enrolment continues and the check phase names the gap. See
   [Secrets and the secrets broker](../concepts/secrets.md) for why these
   files matter.
4. **tailnet** — joins the tailnet and points fleet's laptop link at it. In
   kernel mode the seat dials the laptop directly; in userspace mode traffic
   goes through tailscaled's CONNECT proxy, which fleet expects on port 3130,
   and the phase writes `fleet-proxy.env` with
   `HTTPS_PROXY=http://127.0.0.1:3130`. The phase also writes the `[host]`
   config into `~/.config/fleet/fleet.toml`.
5. **fleet** — installs the binary at **`~/tools/fleet`**. If the existing
   file is not writable it is moved aside to `~/tools/fleet.old` (with sudo
   when available). The install is atomic — `install -m 755` to
   `~/tools/fleet.tmp`, then `mv -f` over `~/tools/fleet` — and happens only
   when the bytes differ. The phase then creates two symlinks:
   `~/tools/bin/fleet` → `~/tools/fleet` and `~/tools/bin/repo-queue` →
   `~/tools/fleet`. The `repo-queue` link is the argv0 alias doing the work;
   there is no second binary.
6. **persist** — writes `~/tools/muse-bootstrap.sh` plus a boot hook that,
   after a sandbox recycle, re-runs `--only tailnet,fleet` under
   `flock -n $HOME/tools/.muse-boot.lock` with a 300 s timeout. Only
   `/home/hatch` survives a recycle, so persistence lives there.
7. **init** — brings up the seat's init stack (brokers, the `gh` shim, the
   laptop channel) from the restored tools. This is the same work
   `fleet init` does; see [Proxy, sessions and recovery](../concepts/proxy.md).
8. **check** — verifies the seat: `fleet version` and `fleet capabilities`
   run and the bridge capability is present; the laptop link is reachable the
   way fleet dials it; `fleet secrets get tailscale.auth-key` (a secret name)
   resolves; `fleet secrets list` answers (the session broker answering is
   the expected path; the `bws` fallback is noted as acceptable); and a
   bridge hello/inbox probe succeeds as the seat's agent.

Source: https://github.com/developing-today/code/blob/main/scripts/muse/bootstrap.sh

## Nix build variants

From the repo root, the flake exposes the fleet packages:

```sh
nix build .#fleet .#fleet-serve .#fleet-connect .#fleet-connect-static
```

`pkgs/fleet/default.nix` returns six attributes:

| Attribute              | Cargo features                                                    | Platform            | Installed alias link | Size gate |
| ---------------------- | ----------------------------------------------------------------- | ------------------- | -------------------- | --------- |
| `fleet`                | default (= full)                                                  | glibc               | —                    | no        |
| `fleet-serve`          | `serve`                                                           | glibc               | `fleet-serve`        | no        |
| `fleet-connect`        | none (`--no-default-features`)                                    | glibc               | `fleet-connect`      | yes       |
| `fleet-brokers`        | `fleet-queue` with `client,queue-broker,secrets-broker`, `--bins` | glibc               | —                    | no        |
| `fleet-static`         | default (= full)                                                  | musl (`pkgsStatic`) | —                    | no        |
| `fleet-connect-static` | none                                                              | musl (`pkgsStatic`) | `fleet-connect`      | yes       |

Facts that matter when choosing one:

- The installed binary is always named `fleet`. A hyphenated name
  (`fleet-serve`, `fleet-connect`) is installed as a _symlink to the same
  binary_; the name narrows the capability surface at invocation time. See
  [The shape of the binary](shape.md).
- The size gate is the connect variant's budget: the stripped binary must be
  at most 2 MiB (2,097,152 bytes) or the build fails.
- `fleet-brokers` builds the three standalone broker artifacts `repo-queue`,
  `queue-broker`, and `secrets-broker` from the `fleet-queue` crate. They are
  the migration's rollback inventory and retire only at the close of the
  multicall epic.
- The static builds carry no ONNX Runtime library path: the search-serve role
  in a static binary needs `ATLAS_ORT_LIB` supplied at runtime. The glibc
  non-connect builds bake the path in.

Source: https://github.com/developing-today/code/blob/main/pkgs/fleet/default.nix

## Verifying an install

Two built-in commands verify any install. Both take no arguments — a trailing
argument, including `--help`, is a usage error (exit 2).

```sh
fleet version
fleet capabilities
```

`fleet version` prints five lines: the version (`fleet 0.1.0`), the build
name (`fleet`, `fleet-serve`, or `fleet-connect`, derived by matching the
compiled capability set against the variant sets — with an annotation when
the invocation name narrowed the surface), the derived version (of the form
`0.1.0-dev+g<rev>`), the source revision (the full commit SHA the build was
cut from), and the enabled capabilities — the _usable_ set for this
invocation, in registry order, or `none`.

`fleet capabilities` prints the same facts as JSON, plus the full domain
registry with, for each domain, its summary, aliases, required capabilities,
and a `state` of `available`, `not-in-build`, `narrowed-by-name`, or
`disabled-by-config`. It prints two distinct lists: `capabilities` (what this
invocation can use) and `compiled_capabilities` (what the binary was built
with). Their difference is exactly the narrowing applied by the invocation
name and the host config. On a full build both lists are the same 15
capabilities and every domain is `available`. The Quickstart chapter,
[Quickstart](quickstart.md), walks through what to do with this output.
