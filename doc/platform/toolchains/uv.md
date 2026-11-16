# uv — Setup Cycle

Verified 2026-10-21: uv 0.12.16 (x86_64-unknown-linux-gnu), exposed as a stub at `~/tools/bin/uv`.

## Download / Install
- uv is resolved through the sandbox dispatcher/omnibin system: `~/tools/bin/uv` is a small stub (auto-created on first resolution) that hands off to `~/tools/dispatcher.sh`, which resolves the pinned binary, GC-roots it in the nix store, and caches the lookup (`~/tools/dispatcher/cache/`, ~16ms hits). There is no separate uv install directory to back up — the dispatcher + stub are the install.
- Same pattern covers many other tools in `~/tools/bin/` (deno, ruff, sops, just, shellcheck, etc.). Typing a missing nixpkgs binary name in bash also works: `command_not_found_handle` in `~/tools/shell-env.sh` routes through the same dispatcher, including exact `program@version` pins (inexact versions fail loudly, exit 127, never substituted).

## Gather secrets
- None for uv itself.

## Configure
- No uv-specific configuration is recorded in TOOLS.md (no custom index URL, no cache-dir override). Python work here defaults to the system python3 (base image, survives recycles).
- Outbound package downloads from scripts must respect the egress proxy env vars; `urllib.request` honours them, `http.client` does not (see index.md).

## Test
```bash
uv --version   # expected: uv 0.12.16 (x86_64-unknown-linux-gnu)
```

## Known quirks & what has NOT worked
- No uv-specific failure is recorded in TOOLS.md. The honest gap: uv is used as a resolved-on-demand utility here, not as the project environment manager of record, so there is no documented `uv sync` / venv workflow for this sandbox yet — record one the first time a project actually depends on it.
- General dispatcher caveat that applies to uv: FUSE-based omnibin shell mode does NOT work in this sandbox (mknod for /dev/fuse is denied, EPERM even as root) — the dispatcher path (resolve → realise → run the real binary) is the working alternative.
