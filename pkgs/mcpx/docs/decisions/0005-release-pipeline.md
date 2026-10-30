# 0005 — Release pipeline, version single-sourcing, and the module path

```
number:   0005
status:   proposed
date:     2026-10-29
issues:   #1881 (this design) · split from triage 2774 · implementation is queue task 3114
code:     7d1b081c
```

> mcpx has no release at all: no tags, no GitHub releases, no release
> workflow, and a version string hard-coded in four places. The install
> page's channels are promises nothing can keep until a release exists.
> This note decides the shape — channel order, the tag and version scheme
> for a package that lives in a monorepo, the static-build policy, and the
> module-path question — so the implementation slices have one answer to
> build to. The module-path recommendation (§4) is the one item Drew may
> want to overrule; everything else follows from the code as it is.

---

## 1. Current state (verified at 7d1b081c)

- No tags and no releases on developing-today/code for mcpx; the only
  mcpx workflow builds and tests (`mcpx.yml`) plus vendorHash repair.
- Version `0.1.0` is hard-coded four times: `package.nix` (`version =`),
  `package.nix` (`-X main.version=0.1.0`), the fallback
  `var version = "0.1.0"` in `cmd/mcpx/main.go`, and
  `ARG VERSION=0.1.0` in `Containerfile`.
- `go.mod` declares `module github.com/dezren39/mcpx` and
  `package.nix`'s `meta.homepage` points at the same repository, which
  returns 404. mcpx actually lives at `pkgs/mcpx` in
  developing-today/code. 328 Go files import by the current path.
- A static build works: the SQLite driver is pure Go, `CGO_ENABLED=0`
  cross-compiles, and a fully static Linux binary was verified in #182's
  container work. The shipped artifacts are not static by choice:
  `package.nix` takes the nixpkgs cgo default and the Containerfile sets
  `CGO_ENABLED=1` deliberately.
- Unix only (`meta.platforms = lib.platforms.unix`; untagged
  `Setpgid`/`Setsid` use). Release targets are therefore the flake's
  three systems: x86_64-linux, aarch64-linux, aarch64-darwin.
- The nix package is more than the binary: it generates the man page and
  completions and wraps the binary with git/deno/bun/node on PATH. A bare
  release binary has no JavaScript runtime; `mcpx run`/`exec` fail until
  the user installs one (`mcpx doctor` says so). Every non-nix channel
  must state this in its install text.

## 2. Channel order

Build in dependency order; each channel consumes the one before it.

1. **GitHub release binaries + checksums.** The foundation: three static
   binaries (`mcpx-<version>-<system>.tar.gz`) and a SHA256SUMS file on a
   GitHub release. Everything else downloads these.
2. **`curl | sh` installer.** Per-user by default (installs to
   `~/.local/bin`, states the JavaScript-runtime requirement at the
   end); a system-wide variant (`sudo`) documented, not default. The
   script verifies the checksum before installing — an installer that
   skips verification teaches users to pipe unverified binaries.
3. **Container image.** The existing Containerfile, published to GHCR by
   the same release workflow, version taken from the release (§3), not
   from a fifth copy of the string.
4. **Nix.** Already works (`nix profile install …#mcpx`); the release
   does not change it beyond the version source (§3).

Later, and not in the first slice: pwsh (needs Windows binaries, which
need the untagged-syscall work), rpm/pkg (repackage the tarballs once
they exist), the agent-prompt and just-the-plugin channels (they ride on
1–2). **Flatpak is not built**: a sandboxed mcpx cannot spawn the
arbitrary servers, runtimes and binaries that are its entire function;
the sandbox would have to be opened until the packaging is a lie. This
records #1881's recommendation against it as a decision.

## 3. Tags and the version's single source

- **Tag scheme: `mcpx/vX.Y.Z`** on developing-today/code. A path-prefixed
  tag is the monorepo-honest scheme: it cannot collide with another
  package's releases in the same repository, and the release workflow
  triggers on exactly this prefix. (If §4's recommendation is adopted,
  Go's own subdirectory-module versioning wants `pkgs/mcpx/vX.Y.Z` tags
  for the module; the release workflow can cut both tags on the same
  commit — one for humans, one for the Go toolchain — rather than
  letting the Go scheme dictate the public one.)
- **Single source: a `pkgs/mcpx/VERSION` file.** `package.nix` reads it
  for both its `version` and its `-X main.version=` ldflag; the
  Containerfile takes `VERSION` as a build argument whose workflow
  callers pass the file's contents (the in-file `ARG` default becomes
  the file's value at release time, and a comment in the Containerfile
  says the file is the source); the release workflow refuses to publish
  when the tag and `VERSION` disagree.
- **`cmd/mcpx/main.go`'s fallback becomes a sentinel** (`0.0.0-dev`), so
  a binary built without ldflags announces that fact instead of
  impersonating a release. The fallback is the one place a literal
  remains, and it is deliberately not a release number.

## 4. The module path — options and recommendation

`github.com/dezren39/mcpx` names a repository that does not exist, so
`go install …@latest` cannot work and `meta.homepage` 404s. Three ways
out:

- **A — Create the repository the path names** (dezren39/mcpx or
  developing-today/mcpx) and move mcpx into it. Cleanest end state:
  normal tags, normal `go install`, its own Pages and releases. Cost:
  mcpx leaves the monorepo it shares tooling, CI and history with; the
  move itself is a project, and this repository's other packages that
  reference mcpx paths must follow. This is Drew's call, not a worker's.
- **B — Rename the module to where the code lives:**
  `github.com/developing-today/code/pkgs/mcpx`. `go install
  github.com/developing-today/code/pkgs/mcpx/cmd/mcpx@pkgs/mcpx/vX.Y.Z`
  then works from the monorepo, tags follow Go's subdirectory scheme,
  and `meta.homepage` points at the real tree. Cost: a mechanical rename
  across 328 Go files plus `package.nix` references — large diff, no
  design content, scriptable in one pass. If A is ever chosen later, the
  same mechanical rename runs again in the other direction; B does not
  close A's door.
- **C — Keep the path, publish a release-only mirror.** Releases are cut
  to a mirror repository that carries the path's name. `go install`
  stays broken (the proxy resolves the module path, not the mirror's
  releases), the homepage stays a lie, and two repositories must be kept
  in sync forever. Recorded for completeness; not recommended.

**Recommendation: B, now.** It makes every other part of this note true
with one mechanical change and no new repository. A remains available
whenever Drew wants mcpx to stand alone; nothing in B makes A harder.

## 5. Static-build policy for releases

Release binaries are `CGO_ENABLED=0` static builds (the #182-verified
configuration). The caveat is recorded where the install page will read
it: on Linux, pure-Go `os/user` parses `/etc/passwd` instead of asking
NSS, so LDAP/SSSD users are not found, and the pure-Go resolver does not
honour `nsswitch.conf`. On macOS the flag changes nothing observable.
The nix package keeps its own (cgo) build: it targets managed systems
where NSS is the point, and it already exists.

## 6. Implementation slices (queue task 3114, in order)

1. This design note.
2. Version single-sourcing (§3): VERSION file, package.nix, Containerfile
   build-arg, the `0.0.0-dev` fallback sentinel.
3. Rename slice (§4-B), if Drew has not overruled it: module path,
   imports, package.nix references, meta.homepage.
4. Release workflow: tag-triggered (`mcpx/v*`) plus `workflow_dispatch`
   snapshot mode that builds the three artifacts and checksums without
   publishing; the snapshot run is the slice's proof.
5. Installer script + a CI exercise (install a snapshot artifact into a
   temp prefix; `mcpx --version` reports the release version).
6. Container publication wiring in the same workflow; update the #1878
   install-page status column to what actually shipped.
