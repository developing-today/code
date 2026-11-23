# Hatch architecture notes — migrated set

These nine notes are the fleet-platform half of the sandbox architecture
knowledge base that lived in dezren39/hatch under `notes/architecture/`.
They moved here on 2026-11-22 at Drewry's direction, following the
2026-11-11 docs migration (code issue #5976): **this copy is canonical**;
the hatch originals stay in place as historical originals, each carrying
a `Canonical:` header pointing here.

The rest of the set stayed in hatch — it describes the sandbox seat
itself rather than the fleet platform: the index
([notes/architecture/README.md](https://github.com/dezren39/hatch/blob/main/notes/architecture/README.md)),
`capabilities.md`, `system-profile.md`, `home-dir-inventory.md`,
`disk-inventory.md`, and `infra-gaps.md`. `nixos-config-review.md` also
stayed — it is a review of the laptop's NixOS configuration, and its
long-term home is a separate question.

## Contents

- [boot-lifecycle.md](boot-lifecycle.md) — Boot vs recycle, the init system, cron scheduling, and the marker protocol
- [nix-persistence.md](nix-persistence.md) — How `/nix` survives recycles with zero copies (symlink + LD_PRELOAD shim)
- [nix-install-flakes.md](nix-install-flakes.md) — Installing Nix single-user with flakes enabled, the wrappers, self-healing
- [omnibin.md](omnibin.md) — What omnibin is, how we use it without FUSE, links (flake, blog, releases)
- [dispatcher-vs-omnibin-run.md](dispatcher-vs-omnibin-run.md) — How our dispatcher differs from omnibin's `nix run`
- [program-access.md](program-access.md) — How every program is on PATH in every exec (wrappers, profile sync, bash wrapper, `@version`)
- [systemd.md](systemd.md) — Running persistent services via systemd units and detached processes
- [telemetry.md](telemetry.md) — Boot logging: human-readable log, compact JSONL, and what we capture
- [mcpx-opencode-plan.md](mcpx-opencode-plan.md) — mcpx ↔ OpenCode integration plan (phase-1 findings; enable in phase 2 only)
