# T3 Code V2 integration: 0.1.2 server + desktop, muse and pi providers

- Status: implemented in this PR; laptop rollout pending (see checklist)
- Date: 2026-10-26T21-18-21Z
- Roadmap item: none
- Issues: none

## Intent

Bring T3 Code (upstream `pingdotgg/t3code`, MIT) up to the first stable
Orchestrator V2 line and wire it into the fleet as a second intelligence /
control surface alongside OpenChamber — driving the same provider CLIs
(Codex, Claude Code, OpenCode 2, Muse Code, Pi, Antigravity) rather than
replacing any of them — with its providers seeded declaratively, its
desktop version-locked to its server, and its Tailscale Serve exposure
actually working.

## State found

- **T3 0.0.45 (the V1 line)** installed as `latestCli.t3`, with the desktop
  GUI taken from nixpkgs (which was on 0.0.45; earlier notes said 0.0.33).
  0.1.2 (2026-10-22) is the first stable release containing Orchestrator V2:
  `delegate_task`, the T3 Code MCP, provider fork/switch, OpenCode 2 +
  Muse Code providers, the ACP registry, the server queue and scheduled
  tasks.
- **Tailscale Serve was broken.** The `t3code` user service ran
  `t3 serve --tailscale-serve --no-browser`, but `tailscale` was not in the
  unit's PATH, so Serve configuration failed with
  `TailscaleCommandExitError` in the journal and T3 never got its HTTPS
  tailnet entry.
- **Stale and dangerous provider instances in the live `settings.json`.**
  The seed script was add-only, so store-path bumps stranded old binaries:
  the live `opencode-v2` instance still pointed at an OpenCode 2.0.23 build,
  and the settings contained an instance pointing at OpenCode **1.18.19** —
  a 1.x binary must never run against the shared OpenCode 2.x session DB.
- **muse and pi were unwired.** Muse Code (T3 driver id literally `muse`
  in 0.1.2) and Pi (driver `pi`) had no instances, no `*_BIN` environment
  and no PATH entries in the T3 unit, even though both CLIs are fleet
  tools. Muse was also stale fleet-wide (1.4.2 vs the stable channel's
  1.4.13).

## What this PR changes

- `pkgs/latest-cli`: `t3` 0.0.45 → **0.1.2** (new tarball URL + hash,
  `sourceRoot` from the version), licence corrected to MIT (upstream is
  MIT; the old `unfree` marking was an error).
- `pkgs/t3code-desktop` (new): the 0.1.2 desktop from the upstream
  `T3-Code-0.1.2-x86_64.AppImage` via `appimageTools.wrapType2`,
  `pname = "t3code-desktop"`, with the upstream `.desktop` file (Exec fixed
  from `AppRun` to `t3code-desktop`) and icons installed into `$out/share`.
  `home/common` binds it as `t3code` in the let block, shadowing
  `pkgs.t3code` for the existing desktop `runCommand` block.
- `pkgs/muse-code`: 1.4.2-R4684.1 → **1.4.13-R6996.1**, all four platform
  hashes taken from the upstream release manifest reached via
  `GET https://api.meta.ai/muse-code/channels/muse-stable`.
- `home/common` — `systemd.user.services.t3code`: PATH gains `muse-code`,
  `pi-coding-agent`, `mcpx`, `latestCli.gh` and **`pkgs.tailscale`** (see
  the broken-Serve finding above); environment gains `MUSE_BIN`, `PI_BIN`
  and `T3CODE_TELEMETRY_ENABLED=false`.
- `home/common` — `systemd.user.services.t3code-seed-providers`:
  environment gains `MUSE_BIN` and `PI_BIN`.
- `pkgs/t3code-seed-providers/seed.sh`:
  - new `muse` instance (driver `muse`, explicit binary from `MUSE_BIN`,
    skipped when unset) and new `pi` instance (driver `pi`, binary from
    `PI_BIN`, enabled only when `~/.pi/agent/auth.json` exists — mirroring
    the grok credentials pattern, seeded as "Pi (no credentials)" when
    disabled);
  - **refresh-on-seed** for the fleet-owned ids (`opencode-v2`,
    `antigravity-acp`, `muse`, `pi`): an existing instance whose
    `config.binaryPath` differs from the environment-provided binary is
    updated in place, preserving everything else;
  - **safety disable**: any existing opencode-driver instance whose
    `binaryPath` points at an OpenCode 1.x build is set `enabled = false`
    with a stderr note (reversible in the UI);
  - contract kept: unparseable settings are never clobbered, the file is
    only written when something changed, and changes are printed to
    stderr. Verified against a fixture shaped like the live settings
    (stale `opencode-v2`, an enabled OpenCode 1.x instance, a UI-created
    custom instance): muse + pi added, `opencode-v2` refreshed, the 1.x
    instance disabled, the custom instance byte-identical, second run a
    no-op.
- `home/common` — `agent-backup` path unit now also watches
  `%h/.config/muse/auth.json` and `%h/.pi/agent/auth.json`.

### Version matrix

T3 Code 0.1.2 provider minimums vs the fleet builds this PR pins / already
carries:

| Provider    | T3 0.1.2 minimum | Fleet build                             |
| ----------- | ---------------- | --------------------------------------- |
| T3 Code     | —                | 0.1.2 (server + desktop)                |
| Codex       | ≥ 0.159          | 0.160.0                                 |
| Claude Code | ≥ 2.1.280        | 2.1.293                                 |
| OpenCode    | ≥ 2.0.18 (2.x)   | 2.0.30 (fleet fork)                     |
| Muse Code   | (driver `muse`)  | 1.4.13-R6996.1                          |
| Pi          | ≥ 0.80.5         | 0.84.3                                  |
| Grok        | ≥ 1.0.13         | (seeded; enabled only with credentials) |

## The coexistence line: T3 and OpenChamber

- **OpenChamber remains the control plane for OpenCode-native fleet
  work**: its managed server on :3000, its quota and session tooling, and
  the fleet's OpenCode configuration (`opencode.jsonc`, route pools,
  proxies) are unchanged by this PR. Nothing here touches `opencode.jsonc`,
  any proxy, or the OpenChamber units.
- **T3 is a second control surface over the same provider CLIs and the
  same OpenCode data dir** (`~/.local/share/opencode`). T3 threads on the
  `opencode-v2` instance are ordinary OpenCode sessions — visible and
  resumable from OpenChamber, and vice versa.
- **Only OpenCode 2.x binaries may touch that DB.** That is why the seed
  disables 1.x instances and why T3 is pointed at the same pinned 2.0.30
  build as the system `opencode`.
- **T3 owns no provider credentials of its own.** It drives the CLIs' own
  logins (Codex, Claude, `muse login`, Pi's `~/.pi/agent/auth.json`, …).
  Signing a CLI in/out affects both control surfaces equally.

## Cross-launch

- **From T3**: Orchestrator V2's `delegate_task` spawns child threads on
  any provider (including `opencode-v2` and `muse`), and the T3 Code MCP
  lets agents create, message and wait on threads across providers.
- **From OpenCode / OpenChamber sessions** (and any shell): the `t3` CLI
  in the user profile drives the T3 server — `t3 thread start|send|show`
  and `t3 project` — so an OpenCode session can start and steer T3 threads.
- **From Muse**: muse reaches OpenCode via the existing muse-proxy
  (port 18747) and reaches T3 as a native provider (driver `muse`); T3
  can also inject session MCP servers into muse sessions.

## The plug-in surface, in T3 terms

- **Provider instances**, seeded declaratively by
  `t3code-seed-providers` (this PR) and editable in the T3 UI.
- **ACP Registry agents**, installable from the T3 UI; `antigravity-acp`
  (Google's `agy_acp_server`) is the local precedent, already seeded.
- **T3 Code MCP injection**: T3 exposes its thread API to agents as an
  MCP server and can inject session MCP servers into provider sessions.
- **Each harness's own MCP / skill configuration**, which T3-launched
  threads inherit: `opencode.jsonc` plus the mcpx "lootbox" servers for
  OpenCode, and the equivalent config of the Claude / Codex / muse / pi
  CLIs. `mcpx` is on the T3 unit's PATH for this reason.

## Rollout + verification checklist (laptop switch — not done by this PR)

1. Build on the laptop and switch (`home-manager` / NixOS as applicable);
   no heavy build is part of this PR's CI-light verification.
2. `t3code-seed-providers` runs before `t3code.service`; check its stderr
   in the journal for the expected added / refreshed / disabled lines.
3. Journal for `t3code.service` shows Tailscale Serve configured — no
   `TailscaleCommandExitError` — and `tailscale serve status` gains the
   T3 HTTPS entry.
4. In T3, providers are healthy: codex, claudeAgent, opencode-v2, muse,
   pi (pi only when signed in), antigravity-acp.
5. OpenChamber on :3000 and the mcpx daemon are still active and
   unaffected; an OpenCode session started in T3 is visible in
   OpenChamber.
6. The 0.1.2 desktop app connects to the server.

**Warnings.** The V1 → V2 move involves a one-way DB import on the T3
server: there is no downgrade path back to 0.0.45 once V2 has imported.
The mobile app must be **T3 Code 2.0** — a V1 client refuses a V2 server
and vice versa, which is also why the desktop in this PR is
version-locked to the server.

## Rollout outcome (2026-10-26, laptop `amd`)

The checklist above was executed the same day. Results:

- The build and switch succeeded; `home-manager-user.service` initially
  FAILED on hand-placed, unmanaged proxy-supervisor files
  (`~/.config/systemd/user/proxy-supervisor.{service,timer}`,
  `~/.config/proxy-supervisor/config.json`, and a `timers.target.wants`
  symlink, all dated 2026-10-24). They were renamed aside
  (`.unmanaged-20261026`), not deleted. Diffing them against this PR's
  generation surfaced the drift fixed in this PR: the live supervisor
  config supervised `commandcode-proxy` and `muse-proxy`, which the
  repo config did not know about.
- With the conflicts aside, activation succeeded. Verified live:
  seed reported `added muse, pi; refreshed binaryPath for
antigravity-acp, opencode-v2; disabled OpenCode 1.x instance(s)
opencode-v1`; the 0.1.2 server logged `Tailscale Serve configured`
  and answered 200 on loopback and at
  `https://amd-6.raptor-pumpkinseed.ts.net/`; projects `~/code` and
  `~/work` were registered with `t3 project add`.
- Shared sessions PROVEN, with a schema caveat: T3 spawns its own
  `opencode serve` (systemd scope, loopback, ephemeral port) from the
  seeded binary, with the standard data dir. The fork's OpenCode 2.x
  writes current sessions to the `session_v2` table of the shared
  `~/.local/share/opencode/opencode.db` (the legacy `session`/`part`
  tables are the old schema's leftovers — do not look there). Two T3
  threads' sessions (`ses_e859ce97…`, `ses_e859b8063…`, directory
  `~/code`) were found in `session_v2`, i.e. in the same store
  OpenChamber's server reads and resumes from.
- `t3 doctor` confirms the one-way V1 → V2 import happened and the
  pre-0.1.0 database (296 KiB) is kept for rollback; it was left in
  place deliberately.
- NOT verified before the revert below: a live thread on the `muse`
  provider (the CLI requires `--provider` and `--model` together).
- At 17:47 CDT a concurrent workstream's home-manager activation
  loop (building `home.activationPackage` to `/tmp/oc-result` and
  running its `activate`) re-applied an older generation, reverting
  the T3 units to 0.0.45 and the supervisor config to five proxies.
  The system-level switch (t3 0.1.2 CLI, muse 1.4.13 in the profiles)
  persisted; only the home-manager state flip-flops. Steady state
  requires this PR merged (so main-based activations produce the
  0.1.2 state) plus one final switch after the concurrent work
  settles.
- Two pre-existing faults surfaced, both unrelated to this PR:
  `libvirtd.service` fails to start on this host (credential-key
  error), which makes `nixos-rebuild switch` exit 4 even when the
  activation itself completes; and the `commandcode-proxy` user unit
  (marked "NOT YET DEPLOYED" in `home/common`) crash-loops against
  the supervisor-owned transient unit on port 18748 whenever any
  home-manager activation starts it.

## Verification done in this PR

- Seed script fixture test (see above): add / refresh / disable /
  preserve / idempotence, the pi no-credentials case, the
  `/opencode-1.` directory form, and the unparseable-settings guard.
- Desktop AppImage hash computed from the downloaded artifact
  (sha256 `cf01e2d167eff9d01c4c1004d31b2c0da9e2522cc94262e41644d1cb15dfdb5c`)
  and confirmed against the release's upstream `SHA256SUMS` (which also
  confirms the server tarball's `7aed625c…`); its contents were inspected
  (desktop file `t3code.desktop`, icons under `usr/share/icons`).
- Muse 1.4.13 hashes taken from the upstream release manifest; the
  downloaded x86_64-linux artifact's sha256
  (`51e97f8de0aa175080e6576cbc055510a95dfe0ededf175c7f6b34fc8e3bf524`)
  verified to equal the manifest's checksum for that artifact.
- Package evaluation (no build): `t3code-desktop-0.1.2` with
  `mainProgram=t3code-desktop`, `muse-code-1.4.13-R6996.1`,
  `t3-0.1.2` with licence MIT and `sourceRoot=t3-0.1.2-linux-x64`.
- Changed `.nix` files formatted with the repo's treefmt / nixfmt and
  parse-checked. No NixOS / home-manager build was run — that happens on
  the laptop per the checklist above.
