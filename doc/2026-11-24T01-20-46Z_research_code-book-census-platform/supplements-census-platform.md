# Census D — supplements / raw census material

Gathered 2026-11-23 from the laptop clone `~/git/code` at HEAD
`44aaf731ad285f25723f9198f7dabed8a2d21992` unless noted. Commands are the
ones actually run (via `~/tools/laptop-ssh.sh`); outputs are trimmed to the
load-bearing lines, never retyped from memory.

## S1. Workflows (`ls .github/workflows/`, `grep -h runs-on:`, `grep -h uses:`)

- 92 workflow files (full list in the census working notes; families:
  agent-backup-tests, agents, atlas-_, auto-format, bin-tests,
  branch-cleanup, bws-freshness, cargo-target-_, cf-container-hello,
  cold-store-reaper, container-images, daily-_, deploy-flake-checks,
  desktop-commander-worker-_, dispatch-health-probe, flake-inputs-update,
  fleet-_, github-_, go-modules-build, goreleaser, id-_, identity-build,
  lab-ops-hub, main-broken, map-build, mcpx-_, message-bridge-deploy,
  mod-tidy, nix-ci, note-submission-pr, openai-cred-watchdog-tests,
  opencode-config-_, org-run, pkarr-worker-_, pm-_, private-laptop-bridge-_,
  production-line-guard, program-tending, proxy-port-fallbacks,
  queue-transport-alert-deploy, ready-for-agent-tests, repo-universe-tests,
  research-workflow, scripts-test, stale-run-sweeper, voice-line-worker-_,
  workflow-lint, worktree-_).
- runs-on counts: 131 `ubuntu-latest`, 14 `ubuntu-24.04`,
  2 `ubuntu-24.04-arm`, 1 `${{ matrix.os }}`, 2 conditional Blacksmith
  expressions (org-run.yml).
- uses counts (top): 124 actions/checkout@v7; 32
  DeterminateSystems/nix-installer-action@3138316… # v23; 25
  actions/setup-node@v7.1.0; 24
  DeterminateSystems/magic-nix-cache-action@84c0677… # v15; 21
  actions/setup-go@v7; 20 oven-sh/setup-bun; 17 actions/setup-python@v7;
  14 actions/upload-artifact@v7; 7 denoland/setup-deno; 2
  actions/deploy-pages@v5; 1 docker/login-action@v4; 1
  goreleaser/goreleaser-action@v7.
- `grep -rn -i "blacksmith|self-hosted|garnix|determinate|magic-nix-cache|cachix" .github/workflows/`:
  no `self-hosted` hits; Blacksmith only in org-run.yml (header: "GitHub-hosted
  ubuntu-24.04 or, for org repos, Blacksmith (blacksmith-4vcpu-ubuntu-2404).
  See docs/placement.md"; setup_runner default "blacksmith").
- Other laptop clones' workflow counts: dezren39-fleet 11, opencode 30,
  openchamber-upstream 34, devtoday-openchamber 22, dezren39/nix 10,
  nix-cache 2, cairn 2, zettel 4, darwin-nix 10.

## S2. Flake head (`head -250 flake.nix`, greps)

```
outputs = inputs: # flake-parts.lib.mkFlake
  let lib = import ./lib inputs; in
  lib.merge [
    rec { inherit lib nixConfig description;
          hosts = import ./nixos/hosts inputs;
          configurations = lib.make-nixos-configurations hosts;
          vm-configurations = lib.make-vm-configurations hosts;
          vmGuests = import ./nixos/vm-guests; # #5772 guest registry + carve-out list
          unattended-installer-configurations = lib.make-unattended-installer-configurations configurations;
          nixosConfigurations = lib.merge [ configurations vm-configurations unattended-installer-configurations ]; }
    lib.make-vim lib.make-clan lib.make-root-apps lib.make-id
    lib.make-mcpx-overlay lib.make-mcpx-modules lib.make-pm-stack
    lib.make-archer-slack lib.make-laptop-backups-checks lib.make-fleet
    lib.make-deploy ];
inputs (selected):
  clan-core.url = "https://git.clan.lol/clan/clan-core/archive/main.tar.gz";
    # TODO: update! way out of date even as of 2026-03
  microvm.url = "github:astro/microvm.nix";
  fleet.url = "github:dezren39/fleet";
  nixpkgs-unstable.follows = "fleet/nixpkgs";  # 2026-11-02 fleet-wide pin comment
  nixpkgs.follows = "nixpkgs-unstable";
  # No master or stable-channel nixpkgs inputs (#3327, Drew directive 2026-10-31)
  # determinate.url = "https://flakehub.com/f/DeterminateSystems/determinate/0.1"; (commented out)
  flake-parts.url = "github:hercules-ci/flake-parts"; # TODO: use this?
  crane.url = "github:ipetkov/crane/01bc1d4…" # #8392 deps-layer comment
nixConfig (flake.nix ~L419-475):
  substituters = [ cache.nixos.org, yazi.cachix.org, hyprland.cachix.org ] (+ commented-out others)
  trusted-users = [ root, @wheel ]; accept-flake-config = true;
  experimental-features incl. flakes, nix-command, ca-derivations, dynamic-derivations,
    fetch-closure, recursive-nix, pipe-operators, verified-fetches …
```

## S3. lib/default.nix landmarks (grep -n)

- L471 `make-clan` (clan lib call L474; meta.name "developing-today" L476;
  machines `{ user = { }; }`; inventory instances sshd / user-root /
  zerotier / user-user; exports nixosConfigurations + clanInternals L524;
  **devShells defined inside make-clan output from L526**).
- L698 `make-root-apps` (eachDefaultSystem; mkApp; just-recipes.json
  generation comment; mcpx callPackage + container variants).
- L946, L1025, L1528 further `apps =` blocks.
- L1566 `make-deploy` (pkgs/deploy callPackage with sourceRev; validation
  fixtures incl. the #6383 malformed-registry shape).
- File ends ~L1943 with the merge list incl. make-clan, make-root-apps,
  make-deploy.

## S4. Clan search

- `find . -iname "*clan*"` (excl .git): only `lib/clan.sh`,
  `lib/clan-installer-flash.sh`. No `machines/` directory.
- Doc mentions: `doc/2026-11-04_deployment-architecture-services-layer.md`,
  `doc/nix-ecosystem-scan-2026-08.md` (§6 clan: 26.05 stable scheme,
  inventory/roles/tags, vars+sops, `clan machines install|update|build`,
  transports p2p-ssh-iroh → internet → wireguard → zerotier → mycelium →
  yggdrasil → tor).

## S5. vm-guests registry (`nixos/vm-guests/default.nix`, verbatim structure)

- Header: "Issue #5772 … pure data: the flake-level record every guest must
  appear in before it may be defined on a host."
- renderer = { libvirt = "nixvirt"; libvirtFallback =
  "flake-generated-libvirt-xml"; ephemeral = "microvm.nix" } (decision
  recorded on #5772, 2026-11-12).
- generationShape = [ domainDefinition, diskImage, nixosSourceRev ].
- guests.prometheus = { class microvm; host amd; config
  nixos/microvm/prometheus; hypervisor qemu; vcpu 2; memMiB 1024; status
  "defined-not-enabled" }.
- carveOuts = [ ]; waivers = [ ];
  waiverRule = "full drift requires a named waiver with owner + reason +
  expiry, recorded in this file".

## S6. Containers

- `grep -rln dockerTools.buildImage|buildLayeredImage|… pkgs lib nixos`:
  pkgs/lab-ops-hub/container-image.nix, pkgs/identity/container-image.nix,
  pkgs/identity/provider-image.nix, pkgs/mcpx/container-image.nix,
  pkgs/generic-containers/{python-runtime,js-runtime,runtime-base}.nix,
  lib/default.nix.
- runtime-base.nix header: "Generic runtime base image for ID and fleet
  services (#6076)… a small POSIX userspace, a CA bundle…, tzdata, writable
  /tmp and /work… deliberately carries no service code of its own."
  buildLayeredImage contents = [ bash coreutils cacert tzdata ],
  User "1000:1000".
- container-images.yml: builds mcpx-container, identity-image,
  identity-provider-image, generic-{runtime,js-runtime,python-runtime},
  lab-ops-hub image; publish-generic job (push-to-main only) pushes GHCR
  v1 + SHA tags; header cites doc/fleet/generic-containers.md §3.

## S7. infra/dns (Terraform + DNS)

- `ls infra/dns/`: main.tf, .terraform.lock.hcl, terraform.tfstate.enc,
  dns_config.yaml, dns_config/default.nix, domains.txt, TODO, plan.sh,
  apply.sh, apply_skip_plan.sh, init.sh, load.sh, save.sh,
  generate_dns_config.sh, delete_all_dns_records.sh,
  delete_tainted_resources.sh, list_tainted_resources.sh,
  cleanup_lock.sh, cleanup_terraform.sh, cleanup_terraform_tfstate.sh,
  compress_logs.sh + tests, logs/.
- main.tf: backend "local"; providers porkbun cullenmcdermott/porkbun
  0.3.0, sops carlpett/sops 1.4.1; data sops_file porkbun (credentials —
  values not reproduced); locals dns_config = yamldecode(dns_config.yaml)["@"];
  resource porkbun_dns_record.record for_each over records.
- domains.txt: 39 lines (1110x.de, 1110x.com, 0x1110.com, 0x111.dev,
  0x1110.dev, 1110x.dev, l0x.dev, grandnag.us, security.cab,
  default.computer, default.properties, rom.how, rom.run, developing.today,
  rom.rest, o3d.dev, printmy.pictures, 79b.us, 69b.us, 64b.us, 32b.us,
  15b.org, 3dtropics.com, eau3d.org, developing-today.com, eau3d.dev,
  79b.org, 64b.org, lair.cloud, s0s.pw, 01b.us, 15b.us, hax.live,
  developingto.day, yak.pub, printmy3dmodel.com, 3dtropic.com, eau3d.com,
  carbonfreezeme.com).
- `git grep -il terraform` = 44 files; outside infra/dns they are docs,
  vendored .agents skills, CNCF records, dependabot/.gitignore.
- `git grep -il opentofu`: CNCF records, fleet CA source docs,
  infra/dns/.terraform.lock.hcl.
- `git grep -il crossplane`: only the two landed CNCF record trees +
  doc/README.md.
- `git grep -il kubevirt` (doc/pkgs/nixos/infra): CNCF records + fleet CA
  source docs only.
- Wrangler configs in tree: 28 files (`git ls-files | grep -i wrangler`).

## S8. Nix cache

- Flake substituters (S2) do NOT include the nix-cache worker.
- `git grep attic|harmonia|nix-serve` (doc/pkgs/lib/nixos/home):
  doc/nix-ecosystem-scan-2026-08.md §5 (caches table + action item 6);
  doc/platform/toolchains/rust.md L35 (Attic follow-up 2026-10-05: atticd
  server built + runs; client unbuildable — nix C++ libs for nix 2.35.2);
  doc/fleet/repo-universe.yaml (dezren39/nix-serve-ng fork entry).
- nix-cache worker primary records: `~/memory/topics/nix-cache.md`
  (2026-10-05; Worker + R2 nix-cache-nars + KV INDEX + D1 nix-cache-db +
  Queue nix-cache-indexing + daily cron; workers.dev deployment; CA demo
  round-trip 1.8s vs 93s; no Durable Objects rationale) and
  pm-stack/design-register-2026-11-11.md entry C1/C1.1/C1.2/C1.3
  (shipped; zstd push; presigned R2 uploads live 2026-10-07; open:
  retention/LRU GC → nix-cache#7; benchmark BENCHMARK.md in
  developing-today/nix-cache — figures not reproduced per scope rule).
- magic-nix-cache in workflows: 24 uses, `use-flakehub: false,
use-gha-cache: true` (container-images.yml instance).

## S9. pkgs/ + cf

- `ls pkgs/ | wc -l` = 90 entries.
- pkgs/cf/default.nix header: "Cloudflare `cf` CLI — Wrangler's successor…
  `cf` entered open beta 2026-09-28 and is not in nixpkgs… Standing
  direction (Drew, 2026-10-20): prefer `cf` for Cloudflare work";
  version 1.0.0-beta.20, buildNpmPackage, vendored package-lock.json.

## S10. Hosts (`nixos/hosts/default.nix`)

- `nixos` host: framework/13-inch/12th-gen-intel, nvme0n1/2t + tmpfs/root.
- `amd` host: framework/13-inch/7040-amd, nvme0n1/4t + tmpfs/root, profiles
  incl. services/lab-tunnel, services/sandbox-link-tunnel,
  services/pm-stack, services/pm-apps, services/power-guard,
  services/openai-cred-watchdog, services/opencode-go-seating-guard,
  services/opencode-watchdog, services/keyring-auto-unlock,
  services/signal-audit, services/archer-slack; "microvm" and "server"
  profiles commented out; lab-tunnel enable comment cites #6455;
  sandbox-link comment cites #10356 ("control path rides it as primary
  transport; Tailscale is the client's fallback").
- nixos/services/ dirs incl. cloudflared, lab-tunnel, sandbox-link-tunnel,
  id-mail, pm-stack, pm-apps.

## S11. Issue states (laptop `fleet gh api`, 2026-11-23)

- #13905 open — "Code Book + Lab Book programme: whole-project book,
  per-book roadmap sections, lab book, Code Space".
- #5772 open — "VMs: declarative guest definitions (nixvirt / microvm.nix)
  - mutable carve-out list".
- #6076 closed — "impl(lab): generic containers for ID and fleet purposes
  (Nix-built)".
- #13774 open — "fleet actions exec: live short-lived exec sessions inside
  Actions runners, coordinator-handed-off".
- #902 open — "impl(dns-registry): recurring DNS backup job, renewal/cost
  tracking, dashboard view of the domain register".

## S12. CNCF pointers (landed in-repo 2026-11-23T22-33-41Z + 23-46-55Z trees)

- platform-and-gitops.md §3 Crossplane (graduated 2025-11-06; verdict
  mostly-no; revisit trigger incl. DNS); §1 Cozystack; corrections list
  (Atlantis CNCF Sandbox 2024-06-18; OpenTofu CNCF Sandbox Apr 2025).
- registries-and-builds.md: GHCR-now verdict + four self-host triggers;
  Zot first self-host step; Harbor graduated/oversized-for-us; Dragonfly
  graduated 2025-10-28, distribution not registry; Artifactory commercial
  "not our shape"; bottom line §§1-4.
- runtimes-and-serverless.md: KubeVirt graduated 2026-11-04, Cozystack VMs
  are KubeVirt underneath; KEDA graduated, "highest value-per-effort …
  after KubeVirt".

## S13. Garnix / Hercules / Hydra searches

- `git grep -in garnix`: TODO.md (garnix-io template links), doc/archive.md
  (garnix.io blog/template links) only — no service usage.
- hercules-ci-effects: flake input only (flake.nix).
- lib/hydra.nix exists (file listing); no running-Hydra record found.
