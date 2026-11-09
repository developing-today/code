# Running mcpx in a container

The Nix-built OCI image, what it deliberately does not contain, and the
measurements made against the retired Containerfile prototype.

```
id:           running-mcpx-in-a-container
since:        pre-release
created:      2026-09-30T04:00:00-05:00
last-updated: 2026-11-05T10:40:00-05:00
status:       implemented
```

The shipped image is a Nix-built OCI artifact exported as
`packages.x86_64-linux.mcpx-container`. It contains the static mcpx binary,
Bun as its one JavaScript runtime, and the certificate bundle needed for
outbound TLS. The Containerfile prototype has been removed; image contents and
dependencies are now declared in `container-image.nix`.

---

## 1. What it is

The image closure is:

| component | Nix input                 | why                                                |
| --------- | ------------------------- | -------------------------------------------------- |
| mcpx      | `pkgs/mcpx/package.nix`    | pure-Go static binary, built from the flake source |
| JavaScript| pinned flake `bun` package | `mcpx exec` and `mcpx run` need a JS runtime       |
| TLS       | `cacert`                   | registry and script HTTPS connections             |

The root flake lock pins the Nixpkgs revision supplying Bun and the image
tools. The image tag comes from `pkgs/mcpx/VERSION`; this image is built from
local source, not from a mutable upstream base-image tag.

The image is built from scratch, runs as uid 1000, and sets an explicit PATH
containing only mcpx and Bun. It has no git; §3 says why that is not a loss.

Build and load the image from the repository root:

```
nix build .#packages.x86_64-linux.mcpx-container
docker load < result
docker run --rm mcpx:0.1.0 --version
docker run --rm -v "$PWD/.mcpx.json:/work/.mcpx.json:ro" mcpx:0.1.0 ls
```

## 2. The JavaScript runtime

**The image ships bun.** `mcpx exec` and `mcpx run` are the point of mcpx, and
neither can do anything without a JavaScript runtime -- `runner.Detect`
returns `no JavaScript runtime found` and the command fails. An image that
cannot run a script is an image that can only do the things a plain MCP client
could already do.

Bun because it is the smallest of the three. The binary-size comparison was
measured on this machine (linux/arm64, inside each runtime's Debian image):

| runtime                           | binary   |
| --------------------------------- | -------- |
| bun 1.4.2                         | 79.4 MB  |
| deno (`denoland/deno:debian`)     | 85.0 MB  |
| node 22 (`node:22-bookworm-slim`) | 122.2 MB |

Smaller margins than the x86-64 figures usually quoted, but the same order.

`runner.Resolve` picks the first of `script.runtimeOrder` (by default **deno, bun, node**) on `PATH`, so an image
with only bun would find bun anyway. `MCPX_SCRIPT_RUNTIME=bun` is set anyway,
because relying on absence to select a default means adding a second runtime
later silently changes which one runs. The setting is `script.runtime`
(`--script-runtime`, `script.runtime` in config); naming a runtime the image
does not contain is an error, not a fallback, so anyone who wants deno's
permission model has to add deno to the image.

That is the one real cost of choosing bun: bun has no permission model, so
any `script.permissions` profile other than `all` is refused under it (see
[runtimes.md](runtimes.md)). Node enforces the `readnet` profile through
`--permission`; deno enforces all of them.
Inside a container that is a smaller gap than it is on a laptop, but it is a
gap.

## 3. No git

The `repo` and `worktree` sharing scopes key server instances by the
repository a call came from. They used to ask git, and the first version of
this image installed git so they could: 105 MB, most of it perl.

They no longer ask git. mcpx reads `.git` itself -- a port of git's own
discovery rules, compared against `git rev-parse` case by case in the test
suite -- and runs git only for the few layouts that port does not vouch for:
a repository format newer than it knows, an extension it has not heard of, a
config file it cannot parse. Without git those key by directory, and `mcpx
doctor` says which file it could not read. The full record, every case and
what was decided for it, is `docs/git-discovery.md`.

Taking git out cost nothing, because in this image it had not been working:

```
$ docker run --rm -v "$PWD:/work:ro" --entrypoint git mcpx:before -C /work/proj \
    rev-parse --git-common-dir
fatal: detected dubious ownership in repository at '/work/proj'
To add an exception for this directory, call:

	git config --global --add safe.directory /work/proj
```

A mounted checkout belongs to the host's uid, the image runs as 1000, and git
refuses a repository someone else owns. So with git installed, the repo and
worktree scopes fell back to per-directory keys for every mounted checkout --
the situation the old version of this section said git was there to prevent.
Native discovery does not check ownership: `safe.directory` exists to stop
git running configuration from a repository someone else controls, and
discovery runs nothing.

What was verified in the image with no git on `PATH`, one daemon for the whole
run, a real repository with a linked worktree (made with
`worktree.useRelativePaths`, so its paths survive the mount) and a submodule,
a bare repository, and a tmpfs mounted inside the checkout:

```
INSTANCE  CALLS  KEY
byrepo#2  4      repo:/work/proj/.git              proj, proj/sub, proj-wt/sub, and exec from proj-wt
byrepo#3  1      repo:/work/proj/.git/modules/mod  the submodule is its own repository
bytree#2  2      worktree:/work/proj               proj and proj/sub
bytree#3  2      worktree:/work/proj-wt            proj-wt/sub and exec
bytree#4  1      worktree:/work/proj/mod
bytree#5  1      cwd:/work/bare.git                no worktree: core.bare is true in /work/bare.git/config
bytree#6  1      cwd:/work/proj/mnt/inner          discovery stops at the mount point /work/proj/mnt
tiny#1    1      global
```

The old image's git, told to trust the mount with `safe.directory=*`, gave
the same answers for the submodule, the bare repository and the mount -- and
refused the other three directories with `unknown repository extension
found: relativeworktrees`, because Debian's git is 2.39 and the checkout was
made by 2.55. With `GIT_DISCOVERY_ACROSS_FILESYSTEM=1` in the
container's environment the mounted directory keys to `worktree:/work/proj`,
as it would in git. `mcpx ls` listed all three namespaces, `call` and `exec`
(on bun) reached each, and `mcpx doctor -v` from the linked worktree said:

```
ok    git   repo /work/proj/.git, worktree /work/proj-wt; resolved natively; no git on PATH,
            which only matters for a repository native discovery cannot read
```

A repository that does need git is visible, not silent: the key falls back to
the directory, the daemon logs why once, and `doctor` turns the line into a
warning that says to install git. Adding it back is one package in the
`apt-get install` line.

## 4. Static linking: the policy

`CGO_ENABLED` is unset in an ordinary Go build, which means _enabled_ on a
native build, and two packages change behaviour when it is:

- `internal/logging/trace.go:9` imports `os/user`, which under cgo calls
  `getpwuid_r` through libc rather than parsing `/etc/passwd`;
- `net` is reachable from most of the tree, and under cgo its resolver is
  `getaddrinfo`, which honours `nsswitch.conf`.

**Policy (#1987): the artifacts that ship as a bare mcpx binary ship
static.** The trade-off is the two bullets above — pure-Go `os/user` does
not consult NSS and the pure-Go resolver ignores `nsswitch.conf` — and it
is accepted for these artifacts because neither behaviour is load-bearing
for a daemon/CLI whose state lives in its own directories, while a binary
that only runs beside one exact glibc (or one Nix store path) fails
everywhere else silently.

| artifact              | static?          | note                                                                                                                                                        |
| --------------------- | ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Nix package           | yes              | `package.nix` sets `env.CGO_ENABLED = "0"`; the installed `bin/mcpx` is a `makeBinaryWrapper` launcher by design — the binary it launches is the static one |
| this image's `mcpx`   | yes              | it is built from the same Go package, with the workstation wrapper disabled                                                      |
| this image as a whole | no               | Bun is dynamically linked; its Nix closure supplies the libraries it needs                                                      |
| release tarball       | not decided here | the release pipeline is #1881's; this policy is the input it should consume                                                                                 |

Measured, not assumed (the CGO_ENABLED=0 build, before it became the
default):

```
$ docker build -f Containerfile --build-arg CGO_ENABLED=0 -t mcpx:cgo0 .
$ docker run --rm --entrypoint sh mcpx:cgo0 -c 'ldd /usr/local/bin/mcpx'
	not a dynamic executable
$ docker run --rm mcpx:cgo0 --version
mcpx 0.1.0
$ docker run --rm -v "$PWD:/work:ro" mcpx:cgo0 \
    exec 'const r = await tiny.echo({ message: "cgo0" }); console.log(String(r));'
you said: cgo0
```

Same size to the byte-ish -- 14 811 298 against 14 811 401 -- so the
pure-Go `os/user` and resolver cost nothing here.

### Tier-2 container legs

Tier 2 (#1987) means: runs nightly and on pushes to main that touch
`pkgs/mcpx`, never gates a PR. The legs are
`scripts/tier2-containers.sh`, run by the `mcpx tier-2 containers`
workflow; each leg asserts the surface its image can actually support,
including mcpx's own clean refusal where a surface is absent:

| leg                         | image                                  | asserts                                                         |
| --------------------------- | -------------------------------------- | --------------------------------------------------------------- |
| `alpine-nonexec`            | `alpine:3.21`                          | `--version`; `exec` refuses with "no usable JavaScript runtime" |
| `alpine-bunmusl-exec`       | alpine + bun's musl build              | `exec` runs a script                                            |
| `scratch-nonexec`           | `FROM scratch` + CA certs              | `--version`; the same clean `exec` refusal                      |
| `distroless-static-nonexec` | `gcr.io/distroless/static-debian12`    | `--version`; the same clean `exec` refusal                      |
| `distroless-cc-deno-exec`   | `gcr.io/distroless/cc-debian12` + deno | `exec` runs a script                                            |

Deferred, with the reason recorded on #1987: an **alpine + nodejs exec**
leg. Verified 2026-10-30, mcpx's node path dies with
`ERR_NO_TYPESCRIPT` under Alpine's nodejs build, so the leg would
enshrine a failure instead of catching one; it joins the matrix when the
node invocation works on a musl node. Tier 3 (fedora, flatpak,
silverblue/kinoite hosts, the Windows port) is unchanged: separate
issues when a user appears.

## 5. Historical prototype size measurements

The following measurements are from the retired Debian-based Containerfile
prototype on linux/arm64. They are not measurements of the current Nix image.

| layer                      | with git                       | without                     |
| -------------------------- | ------------------------------ | --------------------------- |
| `debian:bookworm-slim`     | 108 MB                         | 108 MB                      |
| `apt-get install`          | 105 MB (`ca-certificates git`) | 10.4 MB (`ca-certificates`) |
| bun                        | 79.4 MB                        | 79.4 MB                     |
| mcpx                       | 14.8 MB                        | 14.9 MB                     |
| **layers, uncompressed**   | **307 MB**                     | **213 MB**                  |
| compressed content         | 106 MB                         | 75.6 MB                     |
| `docker images` disk usage | 413 MB                         | 288 MB                      |

The 413 MB this document used to quote is the last row, which is the
uncompressed layers plus the compressed blobs they were unpacked from. The
layers are what a running container occupies; the compressed size is what a
pull transfers.

`git` costing more than bun was not expected: with `--no-install-recommends`
already set, the weight is perl, which Debian's git depends on outright.
What remains of the apt layer is `ca-certificates` and the openssl it pulls
in, for the registry. `perl-base` is still in the image -- it is part of
Debian's essential set, in the base layer -- but nothing in mcpx uses it.

## 6. What was deliberately left out

- **Image build is change-scoped in CI.** The `mcpx nix build` workflow builds
  the Nix image on pull requests touching `pkgs/mcpx/**`, the root flake
  wiring, or that workflow. The broad tier-2 container legs (§4) remain
  nightly and on pushes to main.
- **No full e2e suite inside an image.** The e2e suite spawns daemons and
  binds unix sockets; the tier-2 legs assert surfaces, which is a
  different, smaller harness.
- **No shipped scratch or distroless variant.** They exist as tier-2 test
  legs; a shipped minimal image still drops `exec` (no static JS runtime
  exists to put in it), which is a product decision nobody has taken.
- **No git, Deno or Node.** The image promises Bun as its only JavaScript
  runtime and leaves git discovery fallbacks unavailable, as described in §3.

## 7. What to watch

The image recipe and its `contents` list live in `container-image.nix`; the
root flake exports the image as `mcpx-container` for builds and CI.
