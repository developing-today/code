# Census D — Platform & toolchain census for the Lab Book

- **Programme tracker:** [#13905 — Code Book + Lab Book programme](https://github.com/developing-today/code/issues/13905) (open; title verified via the laptop fleet client, 2026-11-23).
- **Census date:** 2026-11-23 (America/Chicago). Research only — no repo edits were made while gathering; the landing of this record as a dated `doc/` record is the separate step that follows it.
- **Scope (Drew, 2026-11-23 ~19:01 CST):** the platform/toolchain block assigned to the Lab Book — GitHub Actions, Nix/NixOS in general and on the laptop, flake apps, CI/CD from Nix and from Actions, making Nix packages/apps, dev shells, clan.lol, Nix containers, Nix VMs + KubeVirt, Terraform, Crossplane and alternatives, DNS, the Nix cache worker and cache alternatives, next-gen Nix tooling, other build servers, the Artifactory-equivalent question, and the between-lab-and-fleet trio (automation/scale, pipes into Cloudflare and externally, DNS).
- **Status vocabulary (as briefed):** **PROVEN** = a record shows it working/verified · **IN USE** = present in the live estate/config, no separate proof record read · **RESEARCHED** = documents/research exist, no build · **PLANNED** = ticketed/designed, not built · **ABSENT** = searched for, not found (search stated).
- **Method / sources.** (a) The code repo on the laptop (`~/git/code`, HEAD `44aaf731ad285f25723f9198f7dabed8a2d21992`, 2026-11-23): `flake.nix`, `lib/default.nix`, `nixos/`, `.github/workflows/`, `.github/ci-policy.md`, `pkgs/`, `infra/dns/`, `doc/` (incl. `doc/nix-ecosystem-scan-2026-08.md`, `doc/2026-11-04_deployment-architecture-services-layer.md`, the landed CNCF record `doc/2026-11-23T22-33-41Z_research_cncf-landscape/`). Counts below were computed from `ls`/`grep`/`git grep` output in that clone during this census. (b) GitHub issues via the laptop fleet client only (`fleet gh api`): #13905, #5772, #6076, #13774, #902 read directly. (c) The CNCF investigation workspace (`~/workspace/cncf-investigation/research/`) — same text as the landed repo record. (d) Memory: `~/memory/topics/nix-cache.md`, `pm-stack/design-register-2026-11-11.md` (in-repo), and census C (`~/workspace/code-book/census-lab.md`) for cross-references it already verified. (e) The sibling `developing-today/nix-cache` repo as described by its in-repo design-register entry and memory topic (its own clone sits on the laptop at `~/git/nix-cache`).
- **Cost material is out of scope** (Project Management book): where cost records exist they are cited in one line and not reproduced.
- **Headline findings:** (1) The Actions + Nix toolchain is the estate's real, running platform — 92 workflows in this repo, a house pattern (Determinate installer + magic-nix-cache) repeated dozens of times, and a deliberate nightly-on-main CI policy. (2) Terraform is **not** absent — it survives in exactly one place, `infra/dns/`, as the Porkbun DNS tool. (3) The Nix cache worker is **built and proven** but lives in its own repo and is **not** wired into this flake's substituters. (4) Clan is consumed as a flake input with exactly one toy machine defined — a scaffold, not a fleet. (5) KubeVirt, Crossplane, cluster autoscaling and a self-hosted registry are all **researched with verdicts recorded**, none deployed.

---

## 1. GitHub Actions — the estate

**Status: IN USE / PROVEN (it is the CI system of record).**

- **This repo:** 92 workflow files under `.github/workflows/` (counted this census). Shape: per-project suites (id-_, mcpx-_, pm-_, fleet-_, atlas-*), worker publish/promote/deploy workflows, docs-to-Pages workflows, and house-keeping workflows (branch-cleanup, stale-run-sweeper, worktree-janitor/reaper, bws-freshness, flake-inputs-update, daily-flake-sync).
- **Runner estate (counted from `runs-on:` lines):** 131× `ubuntu-latest`, 14× `ubuntu-24.04`, 2× `ubuntu-24.04-arm`, plus matrix/conditional forms. **No `self-hosted` runner label appears anywhere in this repo's workflows** (searched this census).
- **Blacksmith:** present as an _option_, not the default — the reusable workflow `.github/workflows/org-run.yml` ("org run") takes a `runner`/`setup_runner` input choosing GitHub-hosted `ubuntu-24.04` or `blacksmith-4vcpu-ubuntu-2404`; its header records the setup job defaulting to Blacksmith "so matrix expansion off the GitHub-hosted queue", with Blacksmith scoped to org surfaces. The fleet placement design widens this into a spill cascade: **GHA → Blacksmith → Codespaces → QUEUED, never dropped** (`pm-stack/design-register-2026-11-11.md`, fleet execution-surfaces entry). Historical caution of record: a closed fork issue recorded Blacksmith jobs that "never pick up — queue grows unbounded" (opencode fork #46, closed with a decision comment; cited in the design register).
- **Actions-exec:** live short-lived exec sessions inside Actions runners, coordinator-handed-off — [#13774](https://github.com/developing-today/code/issues/13774) **open, in flight** (slices landing 2026-11-23 per the ID-investigation record). The surface exists in the fleet book (`pkgs/fleet/book/reference/actions.md`: `fleet actions exec start/run/attach/watch/renew/extend/decide/close`) and in the queue broker (`pkgs/fleet/crates/fleet-queue/src/client/actions_exec.rs`, broker verbs `/actions-exec-open|bind|poll|io|result`). Design companion: the compile coordinator [#13765](https://github.com/developing-today/code/issues/13765). An audit record (2026-11-24 lane-5) flags that both designs live **only in issue bodies — no `doc/` record**; that gap is on the issues.
- **The forks / wider estate** (workflow counts from the laptop's clones this census; clones other than the primary may lag main): dezren39/fleet 11, developing-today/opencode 30, openchamber 34 (upstream clone) / 22 (devtoday clone), dezren39/nix 10, developing-today/nix-cache 2, cairn 2, zettel 4. The fleet repo's workflows are the sync/placement machinery's own CI; fork-sync pushes fleet main to ~102 forks (memory of record, 2026-11-22).
- **Policy (`.github/ci-policy.md`):** owner rulings 2026-11-13 + 2026-11-16 — no PR/push CI triggers and no required checks; CI runs **nightly on main** (07:17 UTC default) + `workflow_dispatch`; deploys keep push triggers (container-images, mcpx docs Pages, pm-console android). Merges never wait for CI; main CI is post-merge evidence. This matches the standing charter rule (CI on main only).

**Gaps:** no self-hosted runner fleet exists (the "self-hosted" leg is Blacksmith/Actions-exec instead); #13765/#13774 designs lack `doc/` records (already flagged on-issue).

## 2. Nix and the NixOS flake in general

**Status: IN USE (the repo's foundation).**

- **Flake shape:** `flake.nix` is a hand-rolled composition, _not_ a literal flake-parts `mkFlake` call (the header comment `# flake-parts.lib.mkFlake` is aspirational): `outputs = inputs: let lib = import ./lib inputs; in lib.merge [ … ]` over builder functions — `make-clan`, `make-root-apps`, `make-id`, `make-mcpx-overlay`, `make-mcpx-modules`, `make-pm-stack`, `make-archer-slack`, `make-laptop-backups-checks`, `make-fleet`, `make-deploy`, `make-vim`. `flake-parts` is a real input and is used by sub-flakes and inputs (hercules-ci-effects, neovim-nightly, id-flake-parts), but the root flake's composition is `lib/default.nix`'s own `merge` (a deep `recursiveUpdate` fold).
- **`lib/default.nix`** (~1,940 lines): merges nixpkgs + home-manager libs, defines the host/configuration builders (`make-nixos-configurations`, `make-vm-configurations`, unattended-installer configurations), the clan output, dev shells, root apps, deploy checks, and per-project outputs. It is the single most load-bearing file in the platform story.
- **Hosts:** `flake.nix` exposes `hosts = import ./nixos/hosts inputs`, `vmGuests = import ./nixos/vm-guests` (§10), and `nixosConfigurations` = configurations + vm-configurations + unattended-installer configurations.
- **nixpkgs pinning:** `nixpkgs` follows `nixpkgs-unstable`, which since 2026-11-02 follows **the fleet-wide pin in dezren39/fleet** (`fleet.url = "github:dezren39/fleet"`; `nixpkgs-unstable.follows = "fleet/nixpkgs"`) so code, opencode and openchamber build against the same nixpkgs revision and share one store closure. No master or stable-channel inputs (directive #3327, 2026-10-31 — master revs skip Hydra and force source builds).
- **`nixConfig` (in `flake.nix`):** substituters = `cache.nixos.org` + `yazi.cachix.org` + `hyprland.cachix.org` (several more cachix caches present but commented out); a long experimental-features list (flakes, nix-command, ca-derivations, dynamic-derivations, fetch-closure, recursive-nix, pipe-operators, …); `accept-flake-config = true`; trusted users root + `@wheel`. NixOS side: `nixos/nix/settings/default.nix` simply applies `inputs.self.nixConfig` as `nix.settings`.
- **Ecosystem record:** `doc/nix-ecosystem-scan-2026-08.md` is the standing survey (release cadence, nixpkgs structure, module changes, CLI/store, alternatives & caches §5, clan §6) — see §15.

**Gaps:** the root flake's composition style is undocumented as a _pattern_ (no "how this flake is put together" doc was found); several inputs carry "TODO: use this?" / "way out of date" comments (clan-core among them, §8).

## 3. NixOS flake apps

**Status: IN USE.**

- **Root apps are generated from the justfile:** `lib.make-root-apps` (lib/default.nix) reads `just-recipes.json` (produced by `just just-recipes`), filters out private recipes and `default`, and emits one flake app per public recipe — "To add a new nix app: add a recipe to root.just, run `just lockfiles`, and the app appears automatically as `nix run .#<recipe-name>`" (comment of record). Apps are `writeShellScriptBin` wrappers exec'ing `just <name>`, with shared meta (homepage + MIT/ASL20).
- **Hand-written apps also exist** alongside the generated set (further `apps =` blocks in lib/default.nix for project outputs — mcpx, deploy tooling, and the per-system maps at the make-id/make-deploy layers), plus sub-flake apps in `pkgs/id/flake.nix`.
- **Deploy apps:** `just deploy-<name>` → flake app → `deploy.mjs` is the recorded Worker-deploy standard for exactly 3 workers as of the 2026-11-04 deployment-architecture record (§5).

**Gaps:** no catalogue doc lists the current app surface; the generated-from-justfile mechanism is discoverable only from the lib comment.

## 4. The laptop NixOS config specifically

**Status: PROVEN / IN USE (daily driver).**

- Host `amd` in `nixos/hosts/default.nix`: Framework 13-inch 7040-AMD hardware, disks `nvme0n1/4t` + `tmpfs/root`, users user/backup/muse/dot-ssh/root, six wireless profiles, desktop profile plus service profiles: flatpak, id-mail, pm-stack, **lab-tunnel**, **sandbox-link-tunnel**, pm-apps, power-guard, openai-cred-watchdog, opencode-go-seating-guard, opencode-watchdog, keyring-auto-unlock, signal-audit, archer-slack, networking/dhcp-nat, nvidia-egpu specialisation. The `microvm` and `server` profiles exist but are **commented out**.
- Per-host fragments beside it: `amd-rustdesk.nix`, `amd-sunshine.nix`, `amd-krdp.nix`, `amd-ipv4-preference.nix` (the IPv6 workaround, census C §1.4), `amd-openclaw-secrets.nix`, `amd-agent-rebuild.nix` (agent-triggered rebuilds of the amd config on main).
- The second host, `nixos` (Framework 13 12th-gen Intel, 2T NVMe), and a PC Engines APU hardware configuration exist in the same tree (census C §5.1).
- Machine inventory of record: `doc/hardware/machines/amd.md` (collected 2026-10-29 — Ryzen 7 7840U, ~128 GB RAM, Samsung 990 PRO 4TB; census C §5.1).
- Standing mechanics: deploys/rebuilds run from the primary clone on the laptop; the sandbox never builds (charter; CI-first rule).

**Gaps:** none material for a book section — this is one of the best-documented areas; the section should note the commented-out microvm/server profiles as deliberate non-states.

## 5. CI/CD — from Nix, from Actions, and Actions-using-Nix

**Status: IN USE / PROVEN.**

- **Actions-using-Nix is a pinned house pattern** (counted this census): `DeterminateSystems/nix-installer-action` at a pinned SHA (v23) — 32 uses; `DeterminateSystems/magic-nix-cache-action` at a pinned SHA (v15) — 24 uses, typically configured `use-flakehub: false, use-gha-cache: true` (e.g. `container-images.yml`) and often `continue-on-error: true`. Checkout is `actions/checkout@v7` (124 uses). Language setup actions (node/go/bun/python/deno) fill out the rest.
- **What CI does with Nix:** `nix-ci.yml` (nightly full `nix flake check` + advisory `host-eval` — explicitly _not_ a merge gate per ci-policy/#3039), `deploy-flake-checks.yml`, `flake-inputs-update.yml`, `mcpx-nixbuild.yml`, `container-images.yml` (`nix build` of the container packages, §9), `id-check.yml` (comments record the magic-nix-cache rationale: "so unchanged derivations are not rebuilt from cold"; also records Determinate Nix quirks — shallow-repo revCount refusal, lazy-trees note #705).
- **CD from Nix:** `lib.make-deploy` builds `pkgs/deploy` (a deploy-targets registry with schema validation — the validation fixtures in lib/default.nix exist to reject the exact malformed-registry shape of #6383) targeting laptop/systemd services; `pkgs/deploy` is the flake-native deploy leg.
- **CD to Cloudflare:** the 2026-11-04 deployment-architecture record found a split estate — the good pattern (`just deploy-<name>` → flake app → `deploy.mjs`, the 2026-10-31 standard) covered 3 workers (pkarr, developing-today-backup, id-mail), others used raw `wrangler deploy` or a manual REST PUT, **no GitHub Action deployed any Worker** at that date, and id-mail had no rollback path. The current tree shows the follow-through in progress: many `*-worker-publish.yml` / `*-worker-promote.yml` / `*-deploy.yml` workflows now exist (desktop-commander, id-mail, mcpx, message-bridge, pkarr, pm-console-edge, queue-transport-alert, voice-line). The book should present the 2026-11-04 record as the baseline audit and the workflow list as the current state, not conflate them.
- **Releases:** `goreleaser.yml` (goreleaser-action v7) and `mcpx-release.yml` cover the binary-release leg; docs sites deploy via `actions/deploy-pages@v5`.

**Gaps:** Worker rollback/coverage gaps from the 2026-11-04 record are not re-audited in any later record found; whether every worker now follows the standard is **not established**.

## 6. Making Nix packages and apps — house patterns in `pkgs/`

**Status: IN USE.**

- `pkgs/` holds **90 entries** (counted; incl. `default.nix`, `shell.nix`). Patterns visible in the record:
  - `callPackage` composition from lib/default.nix (e.g. mcpx: `pkgs.callPackage ../pkgs/mcpx/package.nix { }`, with variants — a container build calls the same package with `wrapWithRuntimes = false, doCheck = false`).
  - Per-package folders with `package.nix` / `default.nix`, some with their own sub-flake (`pkgs/id/flake.nix`, `pkgs/atlas-search/flake.nix`).
  - `buildNpmPackage` for npm-distributed CLIs with vendored lockfiles — `pkgs/cf` (§13) and `pkgs/wrangler` mirror each other, including the "fix the hash from the build failure" update ritual documented in the package comments.
  - Rust via the `crane` flake input (dependency-layer splitting, #8392) and the id rust overlay; Go packages via the standard builders; shell tools via `writeShellScriptBin`.
  - Test/guard packages as first-class citizens (sigterm-witness, lane-overlap-check, opencode-config-normalize, …).
- Fleet itself is a package family here (`pkgs/fleet` — Rust workspace + book), as are the brokers' clients and the PM stack.

**Gaps:** no single "how to add a package" doc was found in this census; the patterns are consistent but learned from neighbours. `pkgs/default.nix`'s exact role vs lib composition was not fully read — flagged, not asserted.

## 7. Dev shells as a default

**Status: IN USE — with one structural oddity the book should name.**

- The root flake's `devShells` are defined **inside `make-clan`'s output** in lib/default.nix (an historical accident of composition, visible in the source): generated for x86_64-linux, aarch64-linux, aarch64-darwin, x86_64-darwin from `nixpkgs-unstable` + the id rust overlay, `allowUnfree` + Android SDK licence acceptance (the shell carries the Android SDK via `nix-common.nix`, shared with `shell.nix` so `nix develop` and `nix-shell` resolve identical versions). The shell pins a **pristine official Bun 1.4.5 binary** instead of the nixpkgs patchelf repack, with the crash evidence recorded on #13337. clan-cli is among the shell's shared packages.
- Sub-flakes carry their own `devShells.default = pkgs.mkShell { … }` (`pkgs/id`, `pkgs/atlas-search`, the roc platform templates).
- So: dev shells _are_ the default entry for the repo and its big sub-projects, but there is no estate-wide devshell standard doc, and the root shell's placement under make-clan means "the clan output" and "the dev environment" are the same flake branch.

**Gaps:** no `direnv` record was searched for in this census (not asserted either way); no per-language shell catalogue.

## 8. clan.lol and configuring fleets of machines

**Status: IN USE as a scaffold; PLANNED as a fleet system. No clan-managed fleet exists.**

- **Input:** `clan-core.url = "https://git.clan.lol/clan/clan-core/archive/main.tar.gz"` with the flake's own comment: "TODO: update! way out of date even as of 2026-03".
- **`lib.make-clan`** (lib/default.nix): `inputs.clan-core.lib.clan { meta.name = "developing-today"; machines = { user = { }; }; inventory.instances = { sshd (server, all tags; host certificates disabled — the repo has no shared openssh-ca vars), user-root (users module), zerotier (this machine as controller *and* peer), user-user } }`, exporting `nixosConfigurations` + `clanInternals` + `clan` config. Comments record a migration already done once: the instances are "clanServices replacements for the clanModules.* imports that modules/shared.nix used to carry (clanModules was removed upstream; see #159)".
- **That is the whole fleet: one machine named `user`** (a second, `sara`, is commented out). There is no `machines/` directory in the repo (searched: only `lib/clan.sh` and `lib/clan-installer-flash.sh` match `*clan*` outside docs). Helper scripts exist for the installer flow (`lib/clan.sh`, `lib/clan-installer-flash.sh`); clan-cli ships in the dev shell.
- **Research record:** `doc/nix-ecosystem-scan-2026-08.md` §6 — clan version scheme now tracks NixOS releases (26.05 stable), inventory instances + roles/tags model, `vars` secrets generators (sops default), `clan machines install|update|build`, and the layered fallback transports (p2p-ssh-iroh → internet → wireguard → zerotier → mycelium → yggdrasil → tor). The scan's relevance note: "this flake already consumes clan-core; the iroh transport pairs with the iroh findings."
- **Where clan would bite:** the lab machines will run Talos, not NixOS (census C §5.1 — no NixOS definitions exist for them), so clan's plausible estate role is the NixOS machines (laptop-class hosts, future NixOS boxes), not the cluster. No record found states that decision explicitly — **gap**.

**Gaps:** clan-core pin is stale by the repo's own comment; no real machine definitions; no doc explains what clan is _for_ in this estate vs the hand-rolled hosts builder.

## 9. Nix building from-scratch containers

**Status: PROVEN / IN USE.**

- **Mechanism:** `pkgs.dockerTools.buildLayeredImage` throughout — images assembled purely from Nix store paths, no Dockerfile and no upstream base image; that _is_ the from-scratch pattern here.
- **Generic runtimes (#6076, CLOSED):** `pkgs/generic-containers/` — `runtime-base.nix` (bash + coreutils + cacert + tzdata only, `User = "1000:1000"`, writable /work + /tmp, CA bundle via `SSL_CERT_FILE`), `js-runtime.nix`, `python-runtime.nix`. The header states the design: the common denominator of the service images, deliberately carrying no service code.
- **Service images:** `pkgs/mcpx/container-image.nix`, `pkgs/identity/container-image.nix` + `provider-image.nix`, `pkgs/lab-ops-hub/container-image.nix`, plus the mcpx e2e test image built inline in lib/default.nix.
- **Pipeline:** `.github/workflows/container-images.yml` builds all of them on the Determinate+magic-nix-cache pattern and, on push to main only, publishes the generic images to GHCR with v1 + commit-SHA tags (ordering/concurrency rationale recorded in the workflow, from PR #8369 review). Policy home: `doc/fleet/generic-containers.md` §3.
- **Cloudflare-side containers:** `pkgs/cf-container-hello` (a Cloudflare Containers hello project with its own workflow) and the lab-ops-hub container (deployed with its worker) are the edge-container leg.
- Drew's programme rule behind this (2026-11-04, via census C §2.4): Nix-built generic containers for ID and fleet, no hand-maintained Dockerfiles; audit trail #5765 (deployment epic) / #5771 (containers Nix-build audit).

**Gaps:** none for the build story; runtime/rollout proof for the generic images rests on #6076's closed record (census C noted it did not re-verify the proof artefacts).

## 10. Nix-built minimal VMs, the guest registry, and KubeVirt

**Status: PLANNED (registry + one defined guest); KubeVirt RESEARCHED-only, ABSENT as config.**

- **Guest registry:** `nixos/vm-guests/default.nix` — "pure data: the flake-level record every guest must appear in before it may be defined on a host" ([#5772](https://github.com/developing-today/code/issues/5772), **open**). It records the **renderer decision (2026-11-12, on #5772):** libvirt class → **nixvirt** (fallback: flake-generated libvirt XML checked into the guest generation, gap named per guest); ephemeral/agent class → **microvm.nix** (already a flake input; one guest under `nixos/microvm/`).
- **Generation discipline:** a guest generation is exactly the triple _(domainDefinition, diskImage, nixosSourceRev)_; data volumes attach to a generation and never live inside it, so rollback swaps the triple and leaves data untouched. Rationale doc: `doc/2026-11-12_vm-guests-generation-discipline.md`.
- **Actual guests: exactly one** — `prometheus` (microvm class, host `amd`, `nixos/microvm/prometheus`, qemu, 2 vCPU / 1024 MiB), status recorded in the registry itself as **`defined-not-enabled`** (the microvm profile is still commented out in `nixos/hosts/default.nix`). The mutable carve-out list and the full-drift waiver list both start — and remain — **empty**.
- **Ultra-minimal / from-scratch custom VM images:** the microvm.nix model (store-shared, image-per-guest) is the estate's minimal-VM mechanism of record. **No record was found designing custom minimal guest images for KubeVirt specifically** — searched `doc/`, `nixos/`, `pkgs/`, `infra/` for kubevirt: hits are only the CNCF records and fleet CA source documents. **ABSENT as a design; the bridge (Nix-built guest image → KubeVirt) is unbuilt and undesigned.**
- **KubeVirt record (CNCF census, `registries-and-builds` companion `runtimes-and-serverless.md` + `platform-and-gitops.md`):** KubeVirt graduated from the CNCF on 2026-11-04; it is "basically the one way people make VMs in Kubernetes now"; **Cozystack's VM offering is KubeVirt underneath** — so the estate's KubeVirt path, if any, arrives _through_ the planned Cozystack cluster (census C §2), not through hand-written KubeVirt config.

**Gaps:** enabling even the seed guest is unproven; no KubeVirt guest-image design exists; #5772's lifecycle proof is still owed.

## 11. Terraform — the actual role

**Status: IN USE in exactly one corner (DNS); ABSENT everywhere else. The brief's "likely absent" is wrong, and the correction matters.**

- **Where it lives:** `infra/dns/` only. `main.tf`: local backend; providers `cullenmcdermott/porkbun` 0.3.0 and `carlpett/sops` 1.4.1 (credentials read from a sops file — values never in the clear in this census); one resource type, `porkbun_dns_record`, driven by `for_each` over `dns_config.yaml`, which is **generated from Nix** (`infra/dns/dns_config/default.nix` → `generate_dns_config.sh`). Encrypted state is committed (`terraform.tfstate.enc`); shell wrappers provide plan/apply/apply-skip-plan/init/load/save and cleanup scripts (`cleanup_terraform.sh`, `cleanup_terraform_tfstate.sh` — the cleanup tooling's existence says the state juggling is manual and known-awkward). `infra/dns/TODO` records open wishes (alias-as-CNAME handling, multi-config refactor, per-section applies).
- **Search evidence for "everywhere else":** `git grep -il terraform` returns 44 files — after `infra/dns/`, they are documentation mentions, vendored agent skills, the CNCF records, and `.gitignore`/dependabot noise. **No other `.tf` configuration, no Terraform in `pkgs/`, `lib/`, `nixos/`, or any workflow was found.**
- **Context:** the cluster programme's hard rule is "no HashiCorp anything" (census C §2, idea register) — Terraform's survival is as a _registrar-DNS tool that predates/coexists with that rule_, not as platform IaC. The CNCF record covers the successors as research: OpenTofu (Linux Foundation; CNCF Sandbox, April 2025) and Atlantis (CNCF Sandbox) in `platform-and-gitops.md`; Crossplane also ships a Terraform provider (§12). No record proposes migrating `infra/dns` to OpenTofu — **decision gap, currently unowned**.

## 12. Alternatives — Crossplane and the control-plane field

**Status: RESEARCHED (full census, verdict recorded); none adopted.**

- **Primary source:** CNCF investigation, `platform-and-gitops.md` §3 (landed in-repo at `doc/2026-11-23T22-33-41Z_research_cncf-landscape/`; workspace copy `~/workspace/cncf-investigation/research/platform-and-gitops.md`).
- **Crossplane:** CNCF **Graduated 2025-11-06** (the census corrects the older "incubating" assumption). What it is: Kubernetes objects for external infrastructure via Providers + Compositions/XRDs; 2.0 (Aug 2025) made resources namespaced and added Operations. **Verdict for this estate: mostly no — not now.** Reasons of record: our infrastructure is NixOS/Talos declared in Nix and Git, not cloud APIs, so providers have almost no surface; Cozystack already _is_ a Kubernetes-native provisioning layer for databases/buckets/clusters/VMs; fleet already owns dispatch/placement. **Named revisit trigger:** the day we need to provision and continuously reconcile _external_ resources — cloud buckets, **DNS**, GitHub org objects, SaaS tenants — through the same GitOps flow as cluster state.
- **Neighbours in the same census file:** GitOps engines (ArgoCD/Flux — FluxCD is already the planned cluster GitOps, census C), OpenTofu + Atlantis (§11), and the platform frameworks (Cozystack §1 of that file). The file's estate lens is explicit that several famous projects "compete with things we own" (fleet above all).

**Gaps:** the revisit trigger (external-resource reconciliation, DNS named) has no owner or tracking issue found in this census — it connects directly to §13's open classification work (#902 / C6).

## 13. DNS management — how it works today (trio item 3)

**Status: IN USE, split across two systems; classification/backup work PLANNED.**

- **Registrar layer — Porkbun + Terraform + Nix-generated config** (§11): `infra/dns/domains.txt` lists **39 domains** (security.cab, developing.today, 01b.us, lair.cloud, the 1110x/0x111 families, …). Census C recorded a completed first pass of a standing domain register + full DNS-config backup (2026-10-25, described there as 40 domains) with recurring machinery on [#902](https://github.com/developing-today/code/issues/902) (**open** — "recurring DNS backup job, renewal/cost tracking, dashboard view of the domain register"; the cost-tracking element belongs to the PM book and is cited here for existence only). The 39-vs-40 count difference between `domains.txt` and the register description is **flagged, not reconciled** — one of them is stale.
- **Cloudflare layer — zones for the service estate:** `security.cab` is the working service zone: `lab.security.cab` (CNAME to the tunnel's `cfargotunnel.com` address, proxied — recorded in `nixos/services/lab-tunnel/default.nix`), `dispatch.security.cab`, `pm.security.cab`, `mail.security.cab` (+ id-mail route names in `nixos/services/id-mail/worker/config.json`), `sandbox-link.security.cab`. Worker custom domains/routes are declared per-project in the 28 wrangler config files in the tree.
- **Tooling direction:** `pkgs/cf` packages Cloudflare's **`cf` CLI** (Wrangler's successor; open beta 2026-09-28; 1.0.0-beta.20 packaged) — `buildNpmPackage` with a vendored lockfile, _because cf is not in nixpkgs_. Standing direction (Drew, 2026-10-20, recorded in the package): **prefer `cf` for Cloudflare work**; `pkgs/wrangler` stays for existing wrangler.jsonc projects until migrated. Helper wrappers `pkgs/cf/wrangler-authed.py` / `wrangler-secret.py` bridge auth in the meantime.
- **Ruling of record:** `doc/living/turso-cloudflare-integration.md` C6 — "Porkbun stays the deliberate registrar layer; per-domain classification still owed", with the classification checkbox still open in that document.
- **Cloudflare Access:** no dedicated Access policy/configuration record was found in the searches run for this census (tunnels and app-level auth are what the record shows — e.g. the dispatch gateway's expected 401). **ABSENT as a documented system; flagged as a gap, not asserted as unused.**

**Gaps:** per-domain registrar/DNS classification (C6) owed; register count discrepancy; Access undocumented; `infra/dns` TODO list unowned.

## 14. The Nix cache worker, and system alternatives/successors

**Status: the worker is PROVEN (built, verified, live per the 2026-11-11 inventory); alternatives RESEARCHED; no successor chosen; the estate does not consume the worker by default — the section's central tension.**

- **The worker:** repo **developing-today/nix-cache** (its own repo; laptop clone `~/git/nix-cache`, 2 workflows). Architecture (memory topic `~/memory/topics/nix-cache.md`, 2026-10-05; design register entry C1 in `pm-stack/design-register-2026-11-11.md`): a Cloudflare **Worker** front door + **R2** bucket `nix-cache-nars` (the NAR bytes) + **KV** namespace INDEX (hot index) + **D1** `nix-cache-db` + **Queue** `nix-cache-indexing` + daily cron orphan sweep; deployed on workers.dev; upload via a secret path prefix (`nix copy --to` cannot send custom headers); signing keypair named `nix-cache-1`. **Deliberately no Durable Objects** (rationale in its README: cache PUTs are atomic/idempotent and the queue gives verified retried indexing).
- **Proof of record:** a content-addressed demo derivation was built → signed → pushed → deleted locally → **substituted back from the cache alone in 1.8s** (vs a 93s build) with only this substituter + key trusted. Follow-on work shipped: a zstd push path (custom script — `nix copy` hardcodes xz) and presigned R2 upload URLs for all objects (bypassing the 100 MB Worker body limit), live-verified 2026-10-07. A benchmark programme against Cachix/Attic/Garnix exists in that repo (BENCHMARK.md) — _its cost/speed figures belong to the PM book / that repo and are not reproduced here._
- **Open follow-ups of record:** **no retention/LRU GC** — the cron sweeps orphans only (filed as developing-today/nix-cache#7 per the design register); "whatever Drew wants to cache for real was never decided"; the Attic client comparison died on a build problem (atticd server built and ran, but the Attic _client_ links nix C++ libraries unavailable for our nix 2.35.2 — `doc/platform/toolchains/rust.md` follow-up note, 2026-10-05; recorded as a dead end).
- **What the estate actually uses today:** this flake's substituters are cache.nixos.org + two cachix caches (§2) — **the nix-cache worker is not among them**; CI uses magic-nix-cache backed by the GitHub Actions cache (§5). The worker is a proven asset that is not wired in.
- **Alternatives/successors in the record:** `doc/nix-ecosystem-scan-2026-08.md` §5 — Lix's docs steer away from perl nix-serve toward **attic / harmonia / S3+garage**; Mic92's cache-shootout (Apr 2026) benchmarked harmonia, nix-serve(-ng), ncps (proxy cache), attic (sqlite/S3, zstd chunks), snix nar-bridge, plain nginx/S3; **Determinate Systems maintains an attic fork** (zhaofengli's original is slow-moving). The scan's action item 6: "Self-hosted cache: attic (or its DS fork) + S3 looks like the current best-practice stack; `n2p` over iroh is an interesting zero-config LAN/WAN store-sync option." A `dezren39/nix-serve-ng` fork exists in the repo universe (`doc/fleet/repo-universe.yaml`). **No successor/migration decision is recorded** — attic+self-host vs the Cloudflare worker vs status quo is an open choice the book should present as open.

## 15. Nix apps V2 / next-gen Nix tooling

**Status: RESEARCHED (one standing scan); "apps V2" as a named programme is ABSENT.**

- No document or issue found in this census uses a "Nix apps V2" / "next-gen apps" programme name; the flake-apps mechanism of record is §3's justfile generation. If Drew's phrase meant the _tooling_ generation question, the record is:
- **`doc/nix-ecosystem-scan-2026-08.md` §§4–5:** upstream Nix 2.30–2.34 highlights (eval profiler, WAL SQLite caches, `nix flake prefetch-inputs`, HTTP binary-cache compression, `external-builders`, `nix store roots-daemon`, unprivileged daemon via `nix-nswrapper`); the estate's system runs a 2.35 pre-release line (scan + `doc/platform/toolchains/rust.md`). **Lix** 2.95 (fork of CppNix 2.18; no lazy trees, CA-derivations rewrite planned; 8–20% faster than 2.18 per its own materials as cited). **Determinate Nix** — the validated downstream; the estate already depends on Determinate's _installer + cache action_ in CI (§5) while the `determinate` flake input in `flake.nix` sits **commented out**. **snix/Tvix** — Rust reimplementation lineage, present in cache benchmarks via nar-bridge.
- Also in the input graph, marked "TODO: use this?": `omnix`, `nix-output-monitor` (used), `nix-topology`, `nix-search`.

**Gaps:** no evaluation/adoption record for Lix or Determinate Nix _as the system Nix_; the scan is a watch-brief, not a decision. The book section is a research summary + open decision, honestly.

## 16. Other build-server alternatives

**Status: IN USE (fleet placement over Actions/Blacksmith); RESEARCHED (the rest).**

- **What exists:** the fleet placement system is the estate's build-server answer — placement dispatch with a runner choice (`--runner auto|github|blacksmith`, `--push-nix-cache`, container/surface options; fleet book + design register), the spill cascade GHA → Blacksmith → Codespaces → QUEUED (§1), Actions-exec sessions (§1), the compile coordinator design (#13765), and the laptop itself as a build host (the `ghcr.io/developing-today/fleet-build:full` CI runner image; a cargo-target-broker for shared Rust build caches; census C §2.4 records the in-cluster build ambition as PLANNED, waiting on the cluster).
- **Garnix:** never used as a CI/build service — `git grep -i garnix` finds only template-repo links in TODO/archive lists and its name in the nix-cache benchmark comparison. **ABSENT as a system.**
- **Hercules CI:** `hercules-ci-effects` is a flake input (with flake-parts following), but no Hercules CI service usage record was found. **IN USE as a library input only, as far as the record shows.**
- **Hydra:** a `lib/hydra.nix` file exists in the repo; no record of a running Hydra was found in this census — **file present, service ABSENT per the record searched** (flagged as lightly-searched).
- **Cachix** appears as (a) upstream project caches the flake consumes (§2) and (b) a benchmark comparator for the cache worker (§14) — not as the estate's own CI cache (that role is magic-nix-cache/GHA cache today).

## 17. Artifactory-equivalent — artifact/binary repository management

**Status: IN USE (GHCR for containers; the Nix cache for Nix artifacts); the self-host decision is RESEARCHED with a named trigger, deliberately open.**

- **Containers today: GHCR.** `doc/fleet/generic-containers.md` §3 records the registry decision (GHCR, the registry this repo already uses): `ghcr.io/developing-today/code/mcpx` (published 0.1.0 + latest, with the publish run cited in `pkgs/mcpx/docs/install.md`), `ghcr.io/developing-today/fleet-build:full` (CI runner image), `ghcr.io/developing-today/generic-runtime` / `generic-js-runtime` / `generic-python-runtime` (published by container-images.yml, §9). The CNCF registries census adds GitHub's own position as cited there: container storage/bandwidth currently free, public packages free, in-Actions pulls not counted — _pricing detail belongs to the PM book; the structural point is "no cost pressure today."_
- **The CNCF verdict (`registries-and-builds.md`, landed in-repo):** "GHCR is the right registry today, and the trigger for self-hosting is friction, not price." Self-host triggers of record: air-gapped/LAN-local clusters, egress at scale, sovereignty, pull-through caching. **Zot** (CNCF Sandbox; single Go binary, OCI-native, no database, storage dir _is_ the registry) is the named first self-host step — "the registry to stand up the day GHCR friction appears, including as a LAN pull-through mirror". **Harbor** (Graduated) is the governance-heavy answer the census explicitly sizes as too much platform for a one-operator estate today. **Dragonfly** (Graduated 2025-10-28) is _not_ a registry — a P2P distribution layer for later, at much larger node counts. **JFrog Artifactory** itself: commercial, multi-format, "not our shape" per the census.
- **Nix artifacts:** the native answer is the binary-cache plane (§14) — nix-cache worker (proven, unwired) or a future attic-class self-host; there is no separate "artifact repository" for Nix outputs, by design of the ecosystem.
- **Everything else** (npm/cargo/generic file artifacts): **no universal artifact repository is in use or recorded** — packages flow from upstream registries and the repo itself. ABSENT, and the record does not treat it as a problem.

**"Best method for us" as the record supports it:** GHCR now (containers) + a Nix binary cache (Nix artifacts), Zot-on-LAN as the pre-decided first step when triggers fire, Harbor/Artifactory-class systems rejected for now on fit. The Zot step and the cache-plane choice (§14) are **decisions the record leaves open** — the book should present them as queued decisions with triggers, not as settled.

## 18. Trio (1) — Automation and automated scale-up/scale-down

**Status: IN USE for builds and edge workers; RESEARCHED/PLANNED for servers and the cluster.**

- **Builds (IN USE):** fleet placement + queue broker scale build capacity by dispatch — jobs spill across GitHub-hosted runners, Blacksmith, Codespaces, and Actions-exec ephemeral sessions that are created, handed off, renewed and closed programmatically (§§1, 16). That _is_ automated scale-up/down of build capacity, with the queue as the scaler; the laptop joins as a placement surface, and the P9 lab-integration draft would add cluster nodes as overflow (PLANNED, unratified — census C §2.4).
- **Containers/edge (IN USE by platform):** the Cloudflare Workers estate (28 wrangler projects) scales on Cloudflare's platform; the estate's own automation there is deployment/promotion workflows (§5), not scaling logic.
- **Servers (PLANNED):** NixOS hosts are rebuilt/replaced from config (clan's `machines install|update` model is researched, §8; the TALM flow gives the cluster the same shape, census C §2.2). No automated provisioning of new physical/VM servers runs today; the one defined VM guest is not even enabled (§10).
- **Cluster autoscaling (RESEARCHED):** the CNCF runtimes census rates **KEDA** (Graduated, event-driven autoscaling incl. scale-to-zero) "the highest value-per-effort project in this file after KubeVirt" — a candidate for when a cluster exists. No HPA/KEDA/Karpenter configuration exists anywhere in the estate (no cluster). The ID currency-autoscaling design (#5825, `doc/2026-11-16_design_id5825.md`) is _application-level_ scaling design (stateless front-end, scaling triggers) — related thinking, not infrastructure autoscaling; the book must not conflate them.

**Gaps:** no written automation/scaling strategy doc ties these together — this trio section is precisely that missing synthesis; cluster scaling is entirely downstream of the cluster existing.

## 19. Trio (2) — The pipes into Cloudflare and externally

**Status: PROVEN / IN USE.**

- **Cloudflare Tunnels (the inbound pipes):**
  - `lab.security.cab` — the lab's first tunnel, **live** 2026-11-06 ([#6455](https://github.com/developing-today/code/issues/6455) CLOSED): connector on `amd`, NixOS module `nixos/services/lab-tunnel/` (token-file based, token never in the store — #8933; watchdog + edge status service on localhost:8790; DNS CNAME recorded in the module).
  - `sandbox-link.security.cab` — [#10356](https://github.com/developing-today/code/issues/10356): the sandbox → laptop-agent control path rides this tunnel as **primary transport**, Tailscale as fallback (host config comment of record).
  - A reusable `nixos/services/cloudflared/` module (sops-held tunnel credentials) exists for the generic case; its header documents the one-time `cloudflared tunnel login/create` ceremony honestly.
  - Service doors on the zone: `dispatch.security.cab` (the PM dispatch gateway that Actions workflows post to; an outside-probe workflow, `dispatch-health-probe.yml`, watches its public path and files an issue on failure), `pm.security.cab`, `mail.security.cab`.
- **Tailscale (the mesh pipe):** `nixos/tailscale-autoconnect/` (sops auth key, login.tailscale.com, split-DNS via systemd-resolved after a recorded dhcpcd/openresolv SERVFAIL incident); the fleet session proxy rides the tailnet to the laptop (the laptop-agent route every agent in this estate uses); census C records the home-LAN facts.
- **Other meshes in the record:** the clan zerotier instance (§8 — clan's controller+peer on the toy machine) and clan's researched transport ladder incl. wireguard/mycelium/yggdrasil/tor (§8); iroh is the P2P transport of the ID/fleet stack (ecosystem scan §7 — including `n2p`, nix-copy-over-iroh).
- **Egress direction:** #5030's design (census C §2.6) — tunnel/shunt bulk traffic to cheap-egress SaaS (R2 zero-egress among them), only cross the home link with what must; enforcement #8408 open. The dual-exposure direction (Tailscale Serve ↔ private Cloudflare Tunnel, item C9) is **drafted, not ratified** (`doc/living/lab-integration.md`).

**Gaps:** the C9 dual-exposure draft awaits ratification (#8873); egress enforcement is unbuilt; the pipes are documented per-service, not in one network map (nix-topology is an unused input that nominally addresses exactly that).

---

## 20. Section readiness — what the record supports, per Lab Book section

| Lab Book section (as assigned)                            | Readiness                                  | Basis                                                                     |
| --------------------------------------------------------- | ------------------------------------------ | ------------------------------------------------------------------------- |
| GitHub Actions (estate, runners, Actions-exec)            | **Full section**                           | §1 — workflow census, runner counts, org-run/Blacksmith, #13774 in flight |
| Nix and the NixOS flake in general                        | **Full section**                           | §2 — flake/lib/hosts/pin structure all in source                          |
| NixOS flake apps                                          | **Full section**                           | §3 — generation mechanism + deploy apps                                   |
| The laptop NixOS config                                   | **Full section**                           | §4 — host definition + inventory record                                   |
| CI/CD from Nix and from Actions (incl. Actions-using-Nix) | **Full section**                           | §5 — pattern counts, ci-policy, deployment audit baseline                 |
| Making Nix packages and apps (`pkgs/` patterns)           | **Full section** (one flagged unread file) | §6                                                                        |
| Dev shells as a default                                   | **Short section**                          | §7 — real but thin record; root shell's make-clan placement is the story  |
| clan.lol and fleets of machines                           | **Short section**                          | §8 — scaffold + research; no fleet to describe                            |
| Nix from-scratch containers                               | **Full section**                           | §9 — patterns, pipeline, GHCR publish all proven                          |
| Minimal/from-scratch VMs + KubeVirt                       | **Short section**                          | §10 — registry discipline is solid; guests ≈ 0; KubeVirt is research-only |
| Terraform's role                                          | **Short section**                          | §11 — one corner, well-evidenced; the correction _is_ the section         |
| Crossplane & alternatives                                 | **Short section**                          | §12 — one strong census file, verdict + trigger                           |
| DNS management (also trio 3)                              | **Full section**                           | §13 — two live systems + tooling direction; gaps named                    |
| Nix cache worker + cache alternatives                     | **Full section**                           | §14 — proven worker, benchmark record, alternatives scan, open decision   |
| Nix apps V2 / next-gen Nix tooling                        | **Short section**                          | §15 — ecosystem scan only; no adoption decision                           |
| Other build-server alternatives                           | **Short section**                          | §16 — placement is the answer; alternatives are mostly absences           |
| Artifactory-equivalent                                    | **Short section**                          | §17 — GHCR-now/Zot-later verdict is crisp; decision intentionally open    |
| Trio 1: automation & scale up/down                        | **Short section**                          | §18 — builds leg is real; cluster leg is research/plans                   |
| Trio 2: pipes into Cloudflare & externally                | **Full section**                           | §19 — live tunnels, mesh, doors, egress design                            |
| Trio 3: DNS management                                    | = the DNS section above                    | One census entry, per the brief                                           |

**Cross-cutting gaps for the programme (not new issues — for the book's to-do lists):** (1) nix-cache worker not wired into the flake's substituters, and its retention GC unbuilt (nix-cache#7); (2) clan-core pin stale, clan's estate role unstated; (3) #13765/#13774 designs lack `doc/` records (already flagged on-issue); (4) DNS register count discrepancy (39 vs 40) + C6 classification owed (#902); (5) no KubeVirt guest-image design bridging §9/§10; (6) Lix/Determinate-as-system-Nix and attic-vs-worker cache decisions both open and unowned in the record; (7) Cloudflare Access undocumented; (8) the 2026-11-04 Worker-deploy audit has no follow-up re-audit.

_Supplements (raw census material — workflow/runner/action counts, flake & lib excerpts, clan/vm-guests sources, infra/dns listing, grep outputs for terraform/garnix/kubevirt, CNCF excerpt pointers) are filed alongside this report in the landing folder._
