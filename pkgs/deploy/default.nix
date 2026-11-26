# The deploy registry + CLI — issue #5766, slices 1-3 (registry + list;
# unit/plist rendering, --dry-run plans, generation log, status/rollback logic;
# real systemd activation with health-check flip-back) and issue #5770,
# slice 4 (sandbox-class activation + the generic unit restorer).
#
# deployTargets: this repo's service definitions (services.nix) in the
# design §3 shape, with each definition's `package` still a derivation —
# the form renderers and later slices consume.
# deployTargetsJson: the same registry rendered to JSON with `package`
# as its store path; this is what the CLI reads.
# deploy: the CLI package (deploy.py wrapped with the registry path).
{
  lib,
  callPackage,
  python3,
  makeWrapper,
  runCommand,
  writeText,
  cloudflared,
  sourceRev ? "dirty",
}:
let
  fleetPkgs = callPackage ../fleet { inherit sourceRev; };
  definitions = import ./services.nix {
    inherit (fleetPkgs) fleet;
    laptopAgent = callPackage ../laptop-agent/package.nix { };
    mcpx = callPackage ../mcpx/package.nix { };
    routeProxy = callPackage ./route-proxy.nix { };
    quotaRecorder = callPackage ./quota-recorder.nix { };
    # The existing Cloudflare deploy apps (issue #5767, phase 1) — the
    # same callPackage expressions lib/default.nix uses for the
    # id-mail-deploy / pkarr-deploy / developing-today-backup-deploy
    # flake apps, so the registry references the identical derivations.
    idMailDeploy = callPackage ../../nixos/services/id-mail/worker/deploy.nix { };
    pkarrDeploy = callPackage ../../pkgs/pkarr-worker/deploy.nix {
      cf = callPackage ../cf { };
    };
    developingTodayBackupDeploy = callPackage ../../infra/github-backup/deploy.nix { };
    # Issue #5767, phase 2: the remaining in-repo Workers join the
    # registry the same way — each deploy app carries its Worker's
    # flake package (bundle / Worker tree, lockfile-pinned deps).
    coldStoreRehydratorDeploy = callPackage ../../pkgs/cold-store-reaper/worker/deploy.nix {
      coldStoreRehydratorBundle = callPackage ../../pkgs/cold-store-reaper/worker/package.nix { };
    };
    labOpsHubDeploy = callPackage ../../pkgs/lab-ops-hub/deploy.nix {
      labOpsHub = callPackage ../../pkgs/lab-ops-hub/package.nix { };
    };
    cfContainerHelloDeploy = callPackage ../../pkgs/cf-container-hello/deploy.nix {
      cfContainerHello = callPackage ../../pkgs/cf-container-hello/package.nix { };
    };
    # code#6369: the broker outage alert Worker.
    queueTransportAlertDeploy = callPackage ../../pkgs/queue-transport-alert/worker/deploy.nix {
      queueTransportAlert = callPackage ../../pkgs/queue-transport-alert/worker/package.nix { };
      cf = callPackage ../cf { };
      wrangler = callPackage ../wrangler { };
    };
    # Remote MCP connector that sends and receives messages.
    messageBridgeDeploy = callPackage ../../pkgs/message-bridge/worker/deploy.nix {
      messageBridge = callPackage ../../pkgs/message-bridge/worker/package.nix { };
    };
    # PM stack (issue #12768): the same package files the NixOS
    # pm-stack module callPackages into its units; the tunnel's
    # package is nixpkgs cloudflared, as in the module.
    pmAggregator = callPackage ../pm-aggregator/package.nix { };
    pmProjector = callPackage ../pm-projector/package.nix { };
    pmTunnel = cloudflared;
    # Issue #12909: ready-for-agent, packaged from its pinned npm
    # tarballs — the last of the four #12768 services to register.
    readyForAgent = callPackage ../ready-for-agent/package.nix { };
    # Issue #5767, external-Worker slice: agent-inbox, webhook-gateway,
    # and webhook-gateway-sink live in dezren39/hatch-personal (source
    # verdict comment 6974159268), not this repo. Each deploy app
    # carries the pinned hatch-personal rev and stages the worker dir
    # at deploy time (sparse checkout — no full clone, no build, no
    # deploy-time npm install; both workers are dependency-free).
    agentInboxDeploy = callPackage ../external-workers/agent-inbox-deploy.nix { };
    webhookGatewayDeploy = callPackage ../external-workers/webhook-gateway-deploy.nix { };
    webhookGatewaySinkDeploy = callPackage ../external-workers/webhook-gateway-sink-deploy.nix { };
  };
  # The registry names each target's store path without depending on it
  # (code#12738): string context would make building the CLI realize every
  # target package, check phases included, before `deploy` itself exists.
  # The CLI builds a target's package only when that target is deployed,
  # from the derivation recorded in targetDrvsJson.
  jsonDefinitions = lib.mapAttrs (
    name: defn: defn // { package = builtins.unsafeDiscardStringContext "${defn.package}"; }
  ) definitions;
  # name -> .drv path. unsafeDiscardOutputDependency keeps the .drv files in
  # the CLI's closure (so `nix-store --realise` can build from them) without
  # building their outputs.
  targetDrvs = lib.mapAttrs (
    name: defn: builtins.unsafeDiscardOutputDependency defn.package.drvPath
  ) (lib.filterAttrs (name: defn: lib.isDerivation defn.package) definitions);
  registryJson = writeText "deploy-targets.json" (builtins.toJSON jsonDefinitions);
  targetDrvsJson = writeText "deploy-target-drvs.json" (builtins.toJSON targetDrvs);
in
{
  deployTargets = definitions;

  deployTargetsJson = registryJson;

  deploy =
    runCommand "deploy-0.1.0"
      {
        nativeBuildInputs = [ makeWrapper ];
        meta = {
          description = "Deploy CLI (slices 1-3: registry list + validation, unit rendering, dry-run plans, generation log, status/rollback logic, real systemd activation; slice 4: sandbox activation + restore) for the services deployment layer";
          mainProgram = "deploy";
        };
      }
      ''
        makeWrapper ${python3}/bin/python3 $out/bin/deploy \
          --add-flags ${./deploy.py} \
          --set DEPLOY_REGISTRY ${registryJson} \
          --set DEPLOY_TARGET_DRVS ${targetDrvsJson}
      '';
}
