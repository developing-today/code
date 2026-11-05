# Deployment architecture: a services layer outside the system switch

Status: design proposal (2026-11-04). Author: WS-E design pass.
Related: code#5610 (route-proxy retirement gate), code#5343 (laptop-agent),
code#5362 (fleet-brokers), code#5412 (fleet binary epic), code#5703
(quota-recorder broker redesign), darwin-nix#343 (service-enable cascade),
`doc/2026-10-31T23-00-00Z_design_cloudflare_worker_deployment_standard.md`
(the Cloudflare deploy standard this design generalizes).

## 0. The problem, with receipts from 2026-11-04

Everything we run is built with Nix already — or is one hand step away from
it. What we do not have is a way to *deploy* a service without either
(a) switching the whole operating system, (b) pulling a git checkout under a
running process, or (c) placing a file by hand and hoping the next switch,
recycle, or redeploy does not strand it. Today's estate exhibits all three
failure modes at once:

| Mechanism today | What it carries | Failure mode observed |
|---|---|---|
| NixOS system switch (`nixos-rebuild switch --flake ~/git/code#amd`) | The laptop OS **and**, via embedded home-manager, effectively every user service (opencode, openchamber, proxies, brokers, pm-stack, mcpx, atlas-search, openclaw) | A one-service change requires a full system activation; service work queues behind unrelated system work for days |
| home-manager (as a NixOS module in the code flake) | All laptop user units: `home/common/default.nix` (~25 `systemd.user.services`), `nixos/services/pm-stack`, `nixos/services/pm-apps`, `nixos/home/*.nix` | Unit definitions only move when the system generation moves; HM's rollback granularity is the whole user profile, not the service |
| Running from the primary checkout | pm-stack aggregator / projector / dispatch / quota-gatherer / fleet-feed run `bun run` straight out of `~/git/code`, kept "current" by pm-git-sync pulling every 30 s and restarting services on subtree changes | The running service is whatever the checkout happens to be; a redeploy once wiped an unmerged gatherer script mid-incident; "deploys from primary" couples every editor of the repo to production |
| Hand-written unit files in `~/.config/systemd/user/` | `laptop-agent.service` (the Python daemon), `opencode-route-proxy.service`, `rustdesk.service`, charge-producer, codedb-code, opencode-usage-export, and more; dozens of stale `*.backup` predecessors still on disk | **The route-proxy hand unit is broken right now**: `activating/auto-restart`, `ExecMainStatus 127`, because its `ExecStart` script (`~/code/bin/opencode-route-proxy-start.sh`) is not on disk — its retirement was declared in a config comment while the hand unit kept flapping. The laptop-agent's Nix package + module exist (darwin-nix `pkgs/laptop-agent`, PR #1459) but the hand unit is what runs |
| Dueling declarations | darwin-nix exports `homeManagerModules.fleet-brokers` (`services.fleet-brokers.enable` declares laptop-agent + both brokers) — **enabled by no machine config**; the code flake's `home/common/default.nix` (lines 876/889) declares the same brokers again as raw `systemd.user.services` running `fleet serve …` | Two sources of truth for the same daemons; the canonical module is dead code, the live one is the hand-shaped copy |
| Sandbox `install.sh` scripts (hatch-personal) | 8 idempotent installers (`scripts/*/install*.sh`) copy unit files into `/etc/systemd/system`; `~/tools/init.sh` re-runs each on every boot because `/etc` does not survive a recycle | Works, but it is eight bespoke shell scripts standing in for a deployment system; every new sandbox service needs a ninth |
| No installer at all | Sandbox `quota-recorder.service/.timer` exists only as templates in `~/tools/quota-recorder/` | The units vanish on every sandbox recycle and are restored **by hand / agent sweep** (documented 2026-11-01 and 2026-11-02; this boot the `/etc` copies are timestamped ~2 min *after* the init restore batch — a manual catch-up, again) |
| Not a unit at all | Sandbox `fleet proxy serve` is `setsid nohup` from `init-ensure.sh`; sandbox RustDesk is a manual `launch.sh` (Xvfb + SSH forward + nohup) | Invisible to `systemctl`, no restart policy, no status surface |
| Manual Cloudflare deploys | `just deploy-<name>` → flake app → `deploy.mjs` for exactly 3 workers (pkarr, developing-today-backup, id-mail — the 2026-10-31 standard); raw `npm run deploy` (`wrangler deploy`) for lab-ops-hub and cf-container-hello; a manual REST PUT for cold-store-rehydrator; **no GitHub Action deploys any Worker; no Workers Builds config exists; the string `wrangler rollback` appears nowhere** | The good pattern exists but covers a third of the estate; id-mail has **no rollback path at all** (plain `wrangler deploy`, no `--deploy-version`); lab-ops-hub's deploy and its container image push are two separate manual steps |
| Containers, mixed | lab-ops-hub crawler: Nix `dockerTools.buildLayeredImage` → `crane push` → wrangler references the registry digest (the one clean precedent). Same app also has a `Dockerfile` "for Docker machines". mcpx ships a 3-stage `Containerfile` (not Nix). pm-apps runs third-party images digest-pinned via `oci-containers` (grist, pad — correct for software we don't own) while Kaneo/PostgreSQL are compose-managed outside it. **`amd-crd` (Chrome Remote Desktop) has no definition in any repo** — the only trace is a secret *name* in sops | One Nix-built image, several Dockerfile/compose paths, and one production container that exists nowhere as code |
| VMs | `virtualisation.libvirtd.enable = true` on the laptop profile; **zero** libvirt domain definitions, no nixvirt, no XML in any repo. `microvm.nix` is a flake input with exactly one defined VM (`prometheus`), and the `microvm` profile is commented out for `amd` | The hypervisor runs; nothing declarative runs on it |

The pattern underneath: **we have build definitions for almost everything
and deployment definitions for almost nothing.** The gap is not packaging —
it is a services deployment layer.

## 1. Principles

1. **Everything is built by Nix.** A service, a worker bundle, a container
   image, a VM image: all are flake outputs. A Dockerfile or an
   `npm install` at deploy time is a bug, not a shortcut.
2. **The system switch is for the system.** Kernel, drivers, hardware,
   filesystems, base users, sshd, tailscale — these deploy by switch, on
   system cadence. Nothing else may require one.
3. **A service deploys alone, atomically, reversibly.** Deploying service X
   builds X's generation, points X at it, restarts X — and nothing else.
   Rollback is repointing at the previous generation, never rebuilding an
   old tree and never editing the system.
4. **One definition, many renderers.** A service is declared once (package,
   arguments, environment-by-name, ports, user, restart policy, health
   check) and rendered to whatever the host needs: a systemd unit, a
   launchd agent, a Cloudflare deployment, a container pin. The precedent
   already exists in-tree: `pkgs/mcpx/modules/` (`common.nix` options with
   `nixos.nix` / `home-manager.nix` / `darwin.nix` renderers) and the forks'
   openchamber→opencode→mcpx enable cascade (Drew, 2026-11-03). The
   services layer promotes that pattern from "mcpx and the Mac" to
   "everything, everywhere."
5. **Deployments are data.** Every target records its generation history
   (generation id, source rev, artifact digest / CF deployment id, when,
   by whom). `deploy <target> status` reads it; rollback consumes it.
6. **Secrets are resolved at activation, by name, from the brokers** —
   never embedded in a derivation, a unit file, or a deploy script's
   arguments. (The existing CF standard already does this: account id from
   SOPS, token from Bitwarden by name, gathered impurely at deploy time.)
7. **No big bang.** The system flake change for this whole program is
   additive: one `deploy` package and one target-registry output. Services
   migrate one at a time, each independently reversible.

## 2. The three layers

### 2.1 System flake — what stays

The NixOS/Darwin system configuration keeps: kernel and drivers (incl. the
eGPU specialisation), hardware and disk layout, filesystems, base users
and auth, sshd (incl. the MaxStartups tuning), tailscale, libvirtd/dockerd
themselves, the sandbox's platform floor — and **the deploy tool itself**
plus the minimal bootstrap each host needs before `deploy` can run
(laptop: the fleet proxy path and the secrets/queue brokers' *client*
floor; sandbox: `init-ensure` reduced to linking the current service
generation's units, §4.4).

The darwin (Mac) configuration in dezren39/darwin-nix is untouched by this
design except as a consumer: its hand-rolled launchd agents are replaced
by services-layer renderings, per darwin-nix#343's finding that the forks'
modules are the canonical form.

### 2.2 Services layer — what moves

Every daemon and deployable we operate becomes a **deploy target**:

- Laptop: opencode server, OpenChamber, the provider proxies
  (route-proxy, codex/claude-code/commandcode/muse proxies), mcpx,
  secrets-broker, queue-broker, laptop-agent, atlas-search serve,
  pm-stack (aggregator, projector, dispatch, quota-gatherer, fleet-feed,
  digests, pm-git-sync's *payload* services), openclaw gateway/embedding,
  t3code, rustdesk.
- Sandbox: quota-recorder, fleet-fetch, charge-inputs-fetcher,
  airthings-monitor, resource-sampler, sysstat-collector, the two backup
  timers, hatch-autocommit — and `fleet proxy serve` becomes a real unit
  under a target instead of a `nohup`.
- Cloudflare: every Worker (pkarr, developing-today-backup, id-mail,
  lab-ops-hub, cf-container-hello, cold-store-rehydrator, agent-inbox,
  webhook-gateway + sink, zettel/brain-dump workers as they land) and
  every container image (§5).
- OpenCode *sessions* ride on the opencode service target: moving the
  server out of home-manager is what "sessions out of home-manager" means
  operationally — a fork bump becomes `deploy opencode`, not a system
  generation.

### 2.3 Home-manager — its remaining proper role

Home-manager keeps the *user environment*: shell, editor, prompt, dotfiles,
user packages, git config. **It defines no services.** Rationale, from the
inventory: HM-as-NixOS-module means user units inherit the system switch's
cadence and blast radius; HM activation rewrites the whole user profile to
change one unit; its generations roll back *everything the user has*, not
the one service that broke. The service-shaped things HM carries today
(the `systemd.user.services.*` blocks in `home/common/default.nix`, the
`nixos/home/*.nix` unit modules, darwin-nix's unenacted fleet-brokers HM
module) are definitions to be *lifted into* the services layer, renderers
and all — not a second deployment system to keep feeding. Where an HM
module is the canonical definition today (mcpx's `home-manager.nix`
renderer), the services layer imports its option set and drops the HM
activation path.

## 3. The service definition

One option set per service, declared in the flake of the repo that owns
the code (code flake for fleet/pm services; the forks for
opencode/openchamber/mcpx; darwin-nix re-exports for the Mac):

```nix
services.<name> = {
  package = …;                      # a flake package
  entryPoint = "bin/fleet";         # relative to the package
  args = [ "serve" "queue-broker" ];
  environment = { PORT = "4801"; }; # values, or secret references by name:
  secrets = [ "supabase.muse-dev2.service-role-key" ];  # resolved at activation
  user = "user";                    # or a systemd DynamicUser where suitable
  ports = [ 4801 ];
  restart = "on-failure";
  healthCheck = "http://100.99.113.103:4801/health";
  targets = [ "laptop" ];           # hosts/classes this may deploy to
};
```

Renderers consume the option set:

- **systemd** (system or user scope): a unit whose `ExecStart` points into
  the target's generation (§4), rendered at deploy time — not at
  NixOS/HM evaluation time.
- **launchd**: a user agent plist rendered the same way; the mcpx darwin
  module's fixed-path copy trick (`/usr/local/libexec/…`) is the precedent
  where a stable path is genuinely required.
- **Cloudflare**: the definition names the worker, its wrangler config
  source, its bindings, and its secrets-by-name; the renderer is the
  deploy flow of §4.3.
- **Container**: the definition names the image package and the consumers
  (CF container class, oci-containers entry, compose pin) whose pins the
  deploy updates.

The **target registry** is a flake output of developing-today/code
(`deployTargets`) merging this repo's definitions with the forks' —
Drew's rule: plenty belongs in the NixOS flake, and *everything else is in
that flake somewhere too*. The registry is that "somewhere": one output a
`deploy` invocation can enumerate (`deploy list`).

## 4. The `deploy` tool

A single flake-built binary in the code flake (package `deploy`), the
generalization of the three existing `<name>-deploy` flake apps
(`id-mail-deploy`, `pkarr-deploy`, `developing-today-backup-deploy` —
`writeShellApplication`s under `just deploy-<name>` today). Those apps are
the working seed, not waste: `deploy` absorbs their per-target logic
(impure secret gathering, versions/deployments flow, dry-run) behind one
interface.

```
deploy list                     # every target, its host class, its current generation
deploy <target>                 # build + activate the next generation of one target
deploy <target> rollback        # re-activate the previous generation
deploy <target> status          # current/previous generations, source revs, health
deploy <target> --dry-run       # build and report, activate nothing
```

A **generation** is: the built artifact (nix store path / CF version id /
image digest), the source rev it was built from, and the rendered
activation files. Activation per target class:

### 4.1 systemd hosts (laptop; NixOS)

Each target owns a **per-service nix profile**:
`/nix/var/nix/profiles/per-service/<name>` (system scope) or the user
equivalent. `deploy <name>` builds the service package + rendered unit,
installs the generation into the profile (`nix-env --profile … --set`,
the same primitive the amd switch fallback already uses for the whole
system), writes the unit file pointing at the profile's `current` link,
`daemon-reload`, restarts **only that unit**, then runs the health check.
Failure at any step: the profile flips back and the unit restarts on the
previous generation — deploy-rs's magic-rollback idea, with nix profiles
as the mechanism instead of a bespoke one. `rollback` is the same flip
done deliberately. No `nixos-rebuild`, no HM activation, no reboot.

### 4.2 launchd hosts (Mac)

Same profile trick under the user's nix profile; the renderer emits the
agent plist pointing at the generation; activation is
`launchctl bootout`/`bootstrap` of that one agent. This is where the
darwin hand-rolled agents (mcpx's writeShellScript wrapper, the bespoke
OpenChamber port in darwin-nix's `openchamber.nix`) retire to.

### 4.3 Cloudflare (Workers, Pages, containers' worker halves)

Adopts and extends the 2026-10-31 standard verbatim where it works:

- Build: the worker bundle is a **flake package** (pinned toolchain and
  dependencies from the lockfile; no `npm install` against the network at
  deploy time beyond what the derivation already vendored).
- Publish: versions/deployments flow — upload a new version tagged with
  the source rev, create a deployment at 100%. (Where the `cf` tool
  cannot do this and wrangler must, as with developing-today-backup
  today, the target's renderer says so — the interface doesn't change.)
- Rollback: create a deployment at the recorded previous version id
  (`deploy <target> rollback` ≡ today's `just deploy-pkarr
  --deploy-version <id>`, generalized). No rebuild, no re-upload.
- Secrets: set/rotated through the same by-name gathering the current
  deploy apps use; a rollback verifies the target version carries its
  required secret bindings first (the github-backup deploy already does
  this check — it becomes standard).
- Every Worker in §0's table becomes a target, including the two still on
  raw `npm run deploy` and the REST-PUT cold-store-rehydrator; id-mail
  gains the rollback it lacks.

### 4.4 Sandbox targets

The sandbox's constraint is different: `/etc/systemd/system` is ephemeral
and there is no user bus, so units are system units — but the *content*
is persistent (home survives). Design: each sandbox target's generation
lives in the user nix profile exactly as in §4.1. The only bootstrap the
platform still owes is a **generic restorer**: `init-ensure` (or a tiny
successor unit) reads the deploy registry's sandbox manifest — the list
of currently-activated sandbox targets, itself a file in the persistent
home — and links each current generation's rendered unit into
`/etc/systemd/system`, `daemon-reload`, `enable --now`. The eight
bespoke `install.sh` scripts collapse into that one restorer plus
per-service definitions; a service with *no* definition (quota-recorder
today) becomes impossible to strand, because being a target *is* the
definition. `fleet proxy serve` gets a unit and a target; RustDesk's
`launch.sh` becomes a target whose activation does the Xvfb/forward
setup the script does by hand today.

### 4.5 Container targets

`deploy image:<name>` builds the image (§5), pushes it to its registry
(the daemon-less `crane push` flow lab-ops-hub already uses), and updates
the recorded pin wherever the image is consumed (wrangler containers
config, an oci-containers entry, a compose file). Activation of a
container target is the pin update + the consumer's own rollout;
`rollback` restores the previous digest. Status shows current/previous
digests — the `:v5`-style hand tags disappear into the generation log.

### 4.6 Safety properties

- A target's activation touches only that target: one unit, one
  deployment, one pin. The blast radius of a bad deploy is one service.
- Deploy refuses to activate when the build fails or the post-activation
  health check fails (auto-flip back, §4.1; for CF, no deployment is
  created until the version is uploaded and verified deployable — the
  pkarr flow's `?deployable=true` pre-check, standardized).
- Two generations are always retained; `status` is the audit surface;
  the generation log is a file per target (and, later, a fleet-queue /
  Supabase record like everything else fleet-shaped).

## 5. Containers: from scratch, from Nix

Default construction: **`dockerTools.streamLayeredImage` /
`buildLayeredImage` or `nix2container`**, contents = exactly the built
binary's closure — from scratch, no base image; `cacert` and `tzdata`
join the closure only when the binary needs them. The lab-ops-hub
crawler is the working precedent (`pkgs/lab-ops-hub/container-image.nix`:
python + cacert + bash + coreutils, absolute `Cmd`, daemon-less crane
push); this design makes it the *only* pattern:

- The crawler's parallel `Dockerfile` (kept "for Docker machines") is
  deleted once the Nix path is the deploy path — one app, one builder.
- mcpx's 3-stage `Containerfile` is replaced by a Nix image package
  (the binary is already a flake package; the bun sidecar it copies in
  today becomes a closure question the package answers, not a stage).
- Software we **don't** own (Grist, Pad) stays as digest-pinned
  third-party images under `oci-containers` — building those from Nix
  means maintaining someone else's stack. The line is ownership:
  ours → Nix-built from scratch; theirs → pinned digest, tracked as a
  target so bumps and rollbacks are still `deploy` operations.
- Kaneo/PostgreSQL's compose arrangement moves under the same pin
  discipline (compose file rendered from definitions, images as
  targets), ending the split where pm-apps declares two containers and
  a deploy script quietly manages the rest.
- **`amd-crd` gets a definition or a waiver.** A production container
  with no code anywhere is the exact disease this design treats; its
  migration either Nix-builds it (Chrome Remote Desktop's packaging may
  force the "fuller userspace, still Nix-composed" exception — a
  `buildImage` with an explicit contents list, never an apt-get
  Dockerfile) or records it as a named mutable exception in §6's sense.
- Cloudflare's four container classes (CrawlerContainerV2, HelloContainer,
  HelloLite, CrawlerProbe) become image targets; the orphaned legacy
  `CrawlerContainer` application gets an explicit retire-or-keep note
  instead of being "left in place, unused" by default.

## 6. libvirt VMs: declarative by default, mutable by exception

Current state is a blank page with good seeds: libvirtd is enabled,
`microvm.nix` is already a flake input, one microVM (`prometheus`) is
defined but its profile is commented out on `amd`, and the flake already
has `lib.make-vm-configurations` machinery producing per-host VM
variants.

Design:

- **Declarative class (the default):** a guest is a NixOS configuration
  in the code flake + a domain definition generated from it.
  - For libvirt domains: evaluate **nixvirt** (declarative domains,
    networks, pools as Nix config) as the renderer; fallback is
    flake-generated libvirt XML checked into the target's generation.
    Disk images come from the flake (`nixos.qcow2` / image builders),
    so a guest generation = {domain XML, disk image, NixOS rev}.
  - For ephemeral/agent-class guests: **microvm.nix** (already an
    input) — its store-shared, image-per-guest model fits short-lived
    sandboxes better than libvirt pets.
  - `deploy vm:<name>` defines/starts the guest from its current
    generation; `rollback` redefines from the previous {XML, image}
    pair. Data volumes are *attached to* generations, never *in* them.
- **Mutable carve-out class (named, finite):** guests whose disks are
  the data (database hosts, the lab cluster's stateful nodes) may drift
  in data but **not** in OS definition — their NixOS config still comes
  from the flake; only their data disks are exempt from generation
  discipline. Anything that can drift in *both* must be named in the
  design's exception list with a reason and an owner — the list starts
  empty and additions are reviewed like firewall rules. "Mutable" is a
  waiver, never a default.

## 7. Migration path

Ordered by pain already suffered. Each step is independently reversible
(rollback = the old mechanism stays until the new one is proven, then is
removed in the same change that proves it), and **none requires a system
switch to land** — the only system-flake change in the whole program is
adding the `deploy` package and the `deployTargets` output. One named
exception to that additive-only claim: the M2 and M4 ownership handoffs
below are deliberately *subtractive* system-flake changes — they remove
(or durably override) the Home Manager declarations of the migrated
units in the same change that proves each migration, because replacing
a live unit file does not relinquish Home Manager's ownership of it
(code#6251).

**M1 — route-proxy (the hand unit that is broken today).**
Define the route proxy as a services-layer target on the laptop; deploy
it; the rendered unit replaces the hand file. *Acceptance:* `deploy
route-proxy status` shows two generations; `:18750` serves a real
request through the façade route; the hand unit file is removed and a
reboot of the user session does not resurrect it; `deploy route-proxy
rollback` demonstrably returns the previous generation. Side effect:
code#5610's retirement gate becomes honest — retiring the proxy later
is a target removal, not a config edit racing a switch.

**M2 — the fleet three: laptop-agent, secrets-broker, queue-broker.**
Collapse the three declarations (darwin-nix's unenacted HM module, code
HM's raw units, the hand laptop-agent unit) into services-layer
definitions on the code flake (the fleet packages already live there);
cut over one daemon at a time, brokers first (they have health surfaces
and the fleet's own clients verify them), laptop-agent last (its client
surface is the widest — the shim delegates to it). *Acceptance per
daemon:* health endpoint OK on the new generation; one real client call
end-to-end (a `fleet secrets get` by name, a queue round-trip, an agent
exec batch) matches the old path's behavior; old unit files archived,
not deleted, until M2 closes; **an explicit Home Manager
unit-ownership handoff** — the daemon's `systemd.user.services`
declaration in `home/common/default.nix` is removed (or replaced by a
durable override that survives activation) in the same change that
proves the migration, since Home Manager replaces hand-installed units
on activation and would otherwise restore the old broker binaries over
the deployed generation; and **post-activation survival** — after the
cutover, a subsequent ordinary Home Manager / system activation is run
and the migrated daemon is verified to still serve from its deployed
services-layer generation (health endpoint OK, unit not restored from
the Home Manager declaration). This also completes code#5343 phase 3
(declare the agent in Nix) in its corrected form: declared as a
*service*, not as a switch passenger.

**M3 — sandbox quota-recorder (first sandbox target).**
Define quota-recorder as a sandbox target; the generic restorer (§4.4)
replaces the missing installer. *Acceptance:* recycle the sandbox;
with zero hand steps, the units are present, the timer fires, and ticks
land with Supabase acks; remove the "reinstalled by an agent sweep"
entry from the runbook of things-that-happen. Then fold the remaining
seven install.sh services in behind the same restorer, one per target,
deleting each bespoke installer as its target proves out.

**M4 — pm-stack off the checkout.**
Aggregator, projector, dispatch, quota-gatherer, fleet-feed, digests
become targets whose generations are built artifacts (bun services
built by the flake — the packages exist as source; the build becomes a
derivation with pinned dependencies) instead of a live checkout +
pm-git-sync's pull-and-restart. *Acceptance:* a gather tick and an
aggregator read served from a deployed generation while the primary
checkout is deliberately moved to a different rev — the running service
does not notice; pm-git-sync retires or shrinks to a repo-keeping role
with no service restarts; **an explicit Home Manager unit-ownership
handoff** — the PM units' declarations under
`home-manager.users` in `nixos/services/pm-stack/default.nix` are
removed (or replaced by a durable override that survives activation)
in the same change that proves the migration, since a later activation
would otherwise restore the checkout-backed services over the deployed
generations; and **post-activation survival** — after the cutover, a
subsequent ordinary Home Manager / system activation is run and the
migrated PM services are verified to still serve from their deployed
generations. The future M4 implementation issue must carry this
handoff + post-activation acceptance verbatim (noted on epic
code#5765; source: code#6251). This is the largest step and deliberately
last: it touches the fleet's own dashboard/queue surfaces.

**Track C (parallel, Cloudflare):**
C1 — lab-ops-hub, cf-container-hello, cold-store-rehydrator join the
deploy standard (flake deploy apps under `deploy`); lab-ops-hub's image
push and worker deploy become one `deploy lab-ops-hub`.
C2 — id-mail gains `--deploy-version` rollback parity with pkarr and
developing-today-backup; the three original deploy apps are absorbed
into `deploy` without behavior change.
C3 — amd-crd: definition-or-waiver decision executed (§5).

## 8. What this is not

- Not a new platform: `deploy` is a thin Nix-native CLI over primitives
  that already exist in the estate (nix profiles, the versions/
  deployments API, crane). No Kubernetes, no Nomad, no config-management
  overlay.
- Not a switch replacement: the system flake keeps §2.1, and host
  provisioning (nixos-anywhere / clan install territory — clan-core is
  already a flake input with `machines.user`) is out of scope except
  where it consumes the same definitions.
- Not a secrets system: the brokers remain the secrets path; the
  services layer is a *consumer* by name.

## 9. Open questions

1. Sandbox nix is single-user with the symlinked-store/taint-shim
   arrangement — per-service profiles work there, but the restorer's
   privilege shape (it writes `/etc/systemd/system`) needs the same
   care the install.sh fleet gets today from `init.sh`'s root context.
2. Registry merge ownership: when a fork bumps its service definition,
   who republishes `deployTargets` — the code flake pin bump is the
   natural gate, but the deploy tool should refuse a registry whose
   definition rev and package rev disagree.
3. Pages vs Workers as separate CF target classes, or one class with a
   renderer flag (no Pages estate exists today — decide when one does).
4. Generation log home: per-target files are v1; a Supabase table
   (agent domain, next to the queue) is the obvious v2 and would let
   the dashboard show deployment state beside everything else.
5. Darwin: launchd rendering is designed (§4.2) but the Mac is a work
   machine — its targets are Drew-operated; the fleet builds and
   verifies definitions by evaluation only.
