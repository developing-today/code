# Standing auto-update system — design

- Created: 2026-11-24 · Record: `doc/2026-11-24T06-25-30Z_design_auto-update-system/`
- Ordered by: Drew, 2026-11-24 00:22 CST — "ensure all daemons in laptop and muse agent and cloudflare are always automatically kept updated. ideally nixos should update laptop but allow override. perhaps home manager override temporarily and rectified in nixos build. all binaries and services nix pkgs."
- Tracking issue: [#14134](https://github.com/developing-today/code/issues/14134)
- Status: design of record. Slices U1–U7 are named in §7 and are queued separately; this record authorises nothing by itself beyond the standing rule that a gate counts only where it names a hard blocker.

## 1. The guarantee

"Always automatically kept updated" is a standing guarantee, not a one-off update pass. For every daemon, binary, and service in the estate, four things hold:

1. **Pin** — a source of truth names the intended version.
2. **Converge** — an automatic, gated path moves the running thing to the pin, with no human and no lane in the loop.
3. **Verify** — a drift check continuously compares running against pin and reports divergence.
4. **Fail safe** — a failed update never replaces the running version, and the previous version is retained for rollback.

"Updated" is therefore a continuously verified state, not an event that happened once.

## 2. Inventory — how things update today (measured 2026-11-24)

### 2.1 Laptop (amd, NixOS)

- **System.** The flake `github:developing-today/code/main#amd` is already the build source of record ([#12428](https://github.com/developing-today/code/issues/12428)), but rebuilds are run by hand or by an agent. `system.autoUpgrade` is set nowhere in the flake — the only mention is `inspiration/hosts/hydra-auto-upgrade.nix`, reference material. The running generation at inventory time was built 2026-11-23.
- **System services** (docker with the pm-grist / pm-pad containers, id-primary, power-guard, tailscaled, nix-daemon, …) come from the system build: they update exactly when someone rebuilds.
- **~65 user services** — the pm-* family (aggregator, console, dispatch, projector, sentinel, tunnel), queue-broker, secrets-broker, opencode, openchamber, mcpx + mcpx-bridge, laptop-agent, fleet-gh-cache (a systemd user unit since [#14099](https://github.com/developing-today/code/issues/14099)), desktop-commander bridge/remote, atlas-search, sunshine, rustdesk, and more. Their definitions are mixed: some flake-defined, some hand-written unit files under `~/.config/systemd/user`, and at least one (pm-console, via `pm-console-redeploy.timer`) redeploys from a live checkout — the checkout is its pin, not the flake.
- **`~/.local/bin` is mixed.** Store symlinks (atlas-search, bw, bws, desktop-commander) sit beside hand-copied binaries: `fleet` (a raw ~11.7 MB copy with a dozen `fleet.bak-*` predecessors kept by hand), `fff-mcp`, a stale `atlas-search-6788` side copy next to its managed symlink, and vendor installs — `claude` → `~/.local/share/claude/versions`, `codex` → `~/.codex/packages/standalone`, `codebase-memory-mcp` → an npm-global prefix, `clef-proxy` → a script inside a checkout at `~/code/bin`.
- **Partial machinery that exists:** the `flake-inputs-update` workflow bumps the opencode/openchamber flake inputs daily; the daily flake sync (workflow + recurring issues, e.g. #11313) is an agent-driven configuration review that explicitly does not bump owned inputs and does not rebuild the laptop. `fwupd-refresh` handles firmware outside Nix.

### 2.2 Muse sandbox (the agent seat)

- Seat binaries live in `~/.local/share/hatch-recovery/bin` as hand-placed drops with `.sha256` sidecars: fleet (currently the `fleet-static-74eee89` build), tailscaled/tailscale, bws, op, restic. A new fleet build reaches the seat when a lane deploys it and flips the pin (the bootstrap-pin PR pattern, e.g. PR #14053). Nothing converges the seat between deploys.
- The daemons (secrets broker, session/queue broker legs, ts3130 relay, gh cache daemon) are kept **alive** by init-ensure plus the 5-minute heal cron — liveness is automated; versions are not. #14099 added the gh-cache ensure leg. The ENV-HARDEN pass (running concurrently with this design) is doing the current-state work on these scripts; this design builds on its result rather than over it.

### 2.3 Cloudflare

- ~21 Workers carry wrangler configs in the repo: the PM console edge, decision-events, notify-relay, voice-line, the desktop-commander gate, the mcpx gate, fleet hosted-worker, id hosted-worker, message-bridge, pkarr, lab-ops-hub, the atlas worker, github-ci, cold-store-reaper, github-backup, the muse handoff worker, and others.
- Deploy routes are per-Worker and inconsistent: a few have CI publish → promote workflow pairs (desktop-commander, id-mail); the console edge has the estate's best pattern (Actions build → health gate on the deployed commit → route flip; [#14107](https://github.com/developing-today/code/issues/14107) / PR #14124 generalises it across the PM surfaces); the rest deploy by hand (`wrangler deploy` from a checkout) or by one-off lanes. [#14059](https://github.com/developing-today/code/issues/14059) is the standing warning: a deployed PM service sat 4,687 commits behind main with nothing alarming.

### 2.4 Related existing work (cited, not duplicated)

- #14107 / PR #14124 — PM Cloudflare migration design: owns the PM surfaces' pipeline and commit-age drift machinery. This design composes with it; PM Workers join the pipeline through #14107's M-slices.
- #12428 — the laptop rebuild record: the manual rectifier, today.
- #14099 — the gh-cache ensure leg: the sandbox convergence seed.
- #8546 — id program update channels: update flow for id _worlds_. A different plane (product, not estate); out of scope, noted so the names never collide.
- Daily flake sync + `flake-inputs-update` — input freshness. This design consumes their output (fresh pins on main) and adds the missing half: convergence of running systems to those pins.

## 3. The model — pin, converge, verify, on three planes

### 3.1 Laptop: the NixOS build is the rectifier

- **Pin.** The flake on main. Every laptop service — system or user — is defined by the flake (a NixOS module or a home-manager module from the same flake), so one build names every version on the machine.
- **Converge.** A scheduled automatic rebuild: a dedicated systemd timer runs the estate's rebuild path against `github:developing-today/code/main#amd`, in the spirit of `system.autoUpgrade` but behind our own gate script — fetch → build → flake evaluation and checks → switch → post-switch health probes against a named set (brokers answer, the PM stack answers, the tailnet is up). Any gate failure means no switch; the running generation is untouched.
- **Rollback.** NixOS generations are retained by policy (boot entries plus a kept-generations floor). A failed post-switch probe triggers automatic rollback to the previous generation and a loud report. A failed update is a report, never an outage and never a silent retry loop: bounded attempts, then hold and surface.
- **Cadence.** Daily, sequenced after `flake-inputs-update` and the daily flake sync have landed their pins, plus an on-demand trigger. The build may be offloaded (fleet build routes); the switch is always local.

### 3.2 The override protocol — home-manager first, the build rectifies

Drew's shape: an override is allowed — home-manager applies a user-level fix immediately — and the NixOS build rectifies it afterwards. The protocol exists so the fast path never becomes a silent divergence.

- **Record.** Applying an override writes an entry to an override ledger in the repo (slice U4 picks the exact form: one small file per override under `nixos/overrides/`, or a single ledger file). The entry states: what changed (unit / package / config), where (host, user, path), why, the issue link, when, and a **verification probe** that proves the fix is live (a version string, a file hash, a behaviour check). The home-manager apply path is wrapped so the entry is written in the same action. An override with no entry is drift, and drift detection treats it as such.
- **Rectify.** The rebuild cycle reads the ledger before switching. For each open override:
  - If the flake on main now expresses the change (the fix was upstreamed into the system or home-manager modules), the build absorbs it; the post-switch probe verifies the fix survived; the entry closes as _rectified_.
  - If the flake does not yet express it, the change is landed on main first — the recorded override carries everything needed to open that PR — or the switch proceeds only with the override re-applied on top at the home-manager layer, the entry staying open.
- **No silent revert — the hard rule.** If a pending generation would remove a live, recorded fix and neither rectification path is ready, the switch is **held** and the divergence is surfaced loudly (a PM needs-you item plus a comment on the override's issue). A build that quietly reverts a live fix is the one failure this protocol exists to prevent.
- **Expiry.** An override still unrectified after N rebuild cycles (U4 sets N; this design suggests 3) escalates from a report line to a needs-you item. Overrides are a bridge, not a residence.

### 3.3 Sandbox: converge to the pin at safe windows

- **Pin.** The bootstrap pin (commit + sha256) in the repo, as today. The pin is the control point; moving it stays a reviewed PR. (A later slice may automate pin movement behind a CI-green gate; the guarantee does not depend on it.)
- **Converge.** init-ensure gains an update leg, built on the ENV-HARDEN substrate: at boot, and at heal-detected safe windows, each seat binary (fleet, tailscale/tailscaled, bws, op, restic) is compared against the pin. A behind pin is fetched, hash-verified, and swapped at the next safe restart of its daemon — never mid-flight. The secrets broker is never restarted mid-refresh; its swap queues for the next natural restart or the next boot, and the drift report shows "pending swap" in the meantime.
- **Verify.** The heal cycle reports seat versions against the pin as part of the drift census (§4).

### 3.4 Cloudflare: every Worker on the pipeline

- **Pin.** Main — each Worker's source and wrangler config in the repo.
- **Converge.** The console-edge pattern, generalised to every production Worker: CI on merge → build → deploy as a new version → health gate (the version reports the merge commit; the gate requires deployed commit == merge commit) → promote / route-flip. The existing publish/promote workflow pairs (desktop-commander, id-mail) are the seeds. By-hand `wrangler deploy` from a checkout is retired Worker by Worker as each joins. PM Workers join via #14107's M-slices, not via this design's slices.
- **Verify.** Each Worker exposes its deployed commit (a `/health` or version endpoint; Workers without one gain the standard snippet when they join the pipeline). The drift census (§4) compares live commit against main per Worker.

## 4. Drift detection — one census

A single scheduled census (daily) produces one report across the three planes. Per service it lists: pinned version, running version, lag age, state (converged / pending / held / drifted), open overrides, and held updates. Silence means converged; anything else is the report — filed on the tracking issue (or its successor) and summarised to PM needs-you only when something is held, or drifting beyond its window.

The census is read-only. It never fixes; fixing is the converge paths' job. #14107's commit-age machinery is the census's Cloudflare input, not a second system.

## 5. Failure behaviour (all planes)

- Stage, gate, then switch. A failed stage changes nothing that is running.
- Previous versions are retained by explicit policy: Nix generations (a floor), Cloudflare Worker versions (promotion is a pointer move; the old version persists), seat binaries (the previous drop is kept until its replacement passes health).
- Bounded retries, then hold + loud surface. No plane retries forever, and no plane hides a hold in a log nobody reads: holds reach the census and needs-you.

## 6. The package-everything rule — and what contradicts it today

**Rule:** anything that runs on the laptop or in the sandbox is (a) a flake package/module, (b) a repo-source Worker in the Cloudflare pipeline, or (c) on the exception list below, with a reason and an owner. Migration path per offender found in the §2 inventory:

- **fleet** — `pkgs/fleet/default.nix` already exists; the contradiction is deployment, not packaging. U3 makes the laptop's fleet (and the brokers it serves) the flake package — a store symlink, exactly as atlas-search and bws already are — and retires the hand-copied binary and its `.bak` pile. Sandbox seats keep the pin mechanism (§3.3), but the pinned build becomes the same flake package's static output.
- **clef-proxy** — runs from a checkout (`~/code/bin`). Package it, or fold it into a flake-defined service (U7).
- **fff-mcp, atlas-search-6788** — side binaries sitting beside managed ones. Package or delete (U7); atlas-search is already packaged, so its side copy is a deletion once verified unused.
- **codebase-memory-mcp** (npm-global) — package via the flake's node tooling, or exception-list (U7).
- **Vendor self-updaters: claude, codex** — these update themselves through vendor channels and cannot be Nix packages without abandoning those channels. (Packaged builds exist upstream for some of them, but switching is a product decision, not an implementation detail.) Exception-listed: version-probed by the census, lag reported, no forced convergence. Package-or-not is Drew's call, recorded as an open decision on #14134 — not assumed by this design.
- **Docker products (pm-grist, pm-pad)** — images are not flake builds today. The bridge: images pinned by digest in flake-defined config, so a rebuild moves them. Their store programmes (#6816 / #3142) own anything deeper.
- **Firmware (fwupd)** — already auto-refreshing at OS level; out of flake scope. The census notes it as covered-elsewhere.

## 7. Slice map

Named, ordered, each independently landable. Queued by Archer when this design lands — not queued by this record.

- **U1 — Drift census v1.** The read-only pinned-vs-running census across the three planes, as a scheduled report. The foundation: every later slice is verified by it.
- **U2 — Laptop auto-rebuild.** The gated scheduled rebuild+switch from GitHub main, with health probes, generation retention, and hold-and-surface failure behaviour. The rectifier's engine, landed before overrides exist.
- **U3 — Fleet as the flake package.** Laptop fleet + brokers run the `pkgs/fleet` package; the hand-copied binary is retired; the sandbox pin is rebased onto the same package output.
- **U4 — Override protocol.** The ledger, home-manager apply recording, the pre-switch divergence gate (no silent revert), post-switch probes, the escalation window.
- **U5 — Sandbox pin convergence.** The init-ensure update leg plus heal-safe swap windows; seats converge to the pin with no lane involved.
- **U6 — Cloudflare pipeline generalisation (non-PM).** Worker by Worker onto publish → health-gate → promote, until no production Worker deploys by hand; the standard commit-reporting snippet. (PM Workers: #14107.)
- **U7 — Exception burn-down.** Package or formally exception-list each §6 offender; the vendor-updater decision recorded.

## 8. Open decisions

- **Vendor self-updaters (claude / codex):** stay exception-listed, or package them and give up the vendor update channels? — Drew. Needed at U7; nothing earlier depends on it.
- **U2 cadence and window:** daily by default; a set maintenance window vs any-time-behind-gates. — Archer at U2; Drew's word overrides.
- **Override escalation window N:** suggested 3 rebuild cycles. — Archer at U4.
