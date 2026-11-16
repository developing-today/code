# Go — Setup Cycle

Verified 2026-10-21: `go version go1.25.1 linux/amd64` at `~/tools/go/bin/go` (installed 2026-09-29, persistent under `~/tools`).

## Download / Install
- Go 1.25.1 unpacked under `~/tools/go/` — binary at `~/tools/go/bin/go`. Add to PATH as needed:
  ```bash
  export PATH="$HOME/tools/go/bin:$PATH"
  ```
- A dispatcher stub also exists at `~/tools/bin/go`, but the real toolchain directory above is the install of record.

## Gather secrets
- None for the toolchain itself.

## Configure — read this before building anything
- **`/tmp` is a 512M tmpfs — linking a Go binary there fails** with "mapping output file failed: no space left on device". Run builds/tests with:
  ```bash
  mkdir -p /home/hatch/tmp
  TMPDIR=/home/hatch/tmp go build ./...    # btrfs, ~95G free
  ```
  Hit 2026-09-30 running mcpx's e2e tests, which `go build` the binary into a temp dir.
- **`testing.TempDir` reads `GOTMPDIR`, NOT `TMPDIR`** — and caches one parent dir per test process (first call wins; later env changes are ignored). Setting TMPDIR inside a test to redirect temp files silently does nothing. Set `GOTMPDIR` *before the first `t.TempDir` call*. Hit 2026-09-30 fixing mcpx's daemon test isolation (the config walk passed through the real $HOME when the temp tree sat under it).
- Under nix builds on this box, Go derivations need `GOFLAGS=-buildvcs=false`: the nix build dir lives inside the /home/hatch git repo, and `-buildvcs=auto` stamps wrong provenance or fails with `error obtaining VCS status: exit status 128` (see nix.md). Prefer `env.CGO_ENABLED = "0"` in nixpkgs Go derivations so no C toolchain is needed at all (the symlinked-store purity check bites C wrappers — escape hatch `NIX_ENFORCE_PURITY=0`).

## Test
```bash
~/tools/go/bin/go version   # expected: go version go1.25.1 linux/amd64
mkdir -p /home/hatch/tmp && TMPDIR=/home/hatch/tmp go build ./...   # in a real module: expected exit 0, no "no space left" link error
```

## Known quirks & what has NOT worked
- The two temp-dir traps above are the whole story so far — both are silent: one fails only at link time, the other fails *never* (it just uses the wrong directory). Check env before debugging test behaviour that "ignores" TMPDIR.
- Home directory `~/go` exists separately from the toolchain dir — don't confuse GOPATH-style leftovers with the install at `~/tools/go`.
