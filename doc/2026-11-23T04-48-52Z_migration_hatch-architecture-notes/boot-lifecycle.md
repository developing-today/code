# Boot Lifecycle

## Boot vs Recycle

These are different things, though they coincide in this environment:

- **Boot** — the kernel started. Detected via new `/proc/sys/kernel/random/boot_id` and `uptime` resetting.
- **Recycle** — the sandbox was torn down and recreated. Detected via the overlay filesystem being wiped (files in `/`, `/usr/local/bin`, `/etc` are gone).

Every new boot ID observed so far came with a wiped overlay, which is why the terms get used interchangeably. But a boot doesn't *have* to wipe the overlay — don't assume it.

## The Init System

`~/tools/init.sh` is the boot orchestrator. It restores all ephemeral state:

1. **nix-ensure** — recreates `/nix` symlink, rewrites `/etc/nix/nix.conf`, verifies nix
2. **nix-wrapper** — recreates `/usr/local/bin/nix` (sets LD_PRELOAD)
3. **nix-profile-sync** — symlinks profile binaries to `/usr/local/bin`
4. **bash-wrapper** — replaces `/bin/bash` with wrapper that auto-loads `@version` handler
5. **go-check / rust-check** — verifies toolchains

### Scheduling

The `sandbox-boot-init` cron fires every 5 minutes. `init.sh` is idempotent:

- On first run per boot: does full restore, writes marker file (`~/logs/.init-boot-id`)
- On subsequent runs: checks marker + `nix_works()`, exits 0 immediately if healthy ("already initialized")

This means worst-case recovery after a recycle is ~5 minutes.

### The Marker Protocol

- Marker file: `~/logs/.init-boot-id` contains the boot ID of the last successful init
- Only written on `FAILURES=0` — failed boots retry on the next 5-minute tick
- The `nix_works()` check uses `/usr/local/bin/nix` (the wrapper), not the raw binary path, because the raw path trips nix's symlink guard without LD_PRELOAD

### What Survives

| Survives | Location |
|---|---|
| Home directory | `/home/hatch` (btrfs, 100GB) |
| Logs | `/home/hatch/logs/` |
| Tools/scripts | `/home/hatch/tools/` |
| Nix store | `/home/hatch/nix-persist/` |

| Wiped | Location |
|---|---|
| System binaries | `/bin/`, `/usr/bin/` (except via init) |
| Local binaries | `/usr/local/bin/` (restored by init) |
| System config | `/etc/` (restored by init) |
| The `/nix` symlink | `/nix` (restored by init) |
| Systemd units | `/etc/systemd/system/` (restored by init if needed) |
