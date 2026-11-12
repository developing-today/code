# The deploy registry + CLI — issue #5766, slices 1-3 (registry + list;
# unit/plist rendering, --dry-run plans, generation log, status/rollback logic;
# real systemd activation with health-check flip-back).
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
  sourceRev ? "dirty",
}:
let
  fleetPkgs = callPackage ../fleet { inherit sourceRev; };
  definitions = import ./services.nix {
    inherit (fleetPkgs) fleet;
    mcpx = callPackage ../mcpx/package.nix { };
    routeProxy = callPackage ./route-proxy.nix { };
    # The existing Cloudflare deploy apps (issue #5767, phase 1) — the
    # same callPackage expressions lib/default.nix uses for the
    # id-mail-deploy / pkarr-deploy / developing-today-backup-deploy
    # flake apps, so the registry references the identical derivations.
    idMailDeploy = callPackage ../../nixos/services/id-mail/worker/deploy.nix { };
    pkarrDeploy = callPackage ../../pkgs/pkarr-worker/deploy.nix {
      cf = callPackage ../cf { };
    };
    developingTodayBackupDeploy = callPackage ../../infra/github-backup/deploy.nix { };
  };
  jsonDefinitions = lib.mapAttrs (name: defn: defn // { package = "${defn.package}"; }) definitions;
in
{
  deployTargets = definitions;

  deployTargetsJson = writeText "deploy-targets.json" (builtins.toJSON jsonDefinitions);

  deploy =
    runCommand "deploy-0.1.0"
      {
        nativeBuildInputs = [ makeWrapper ];
        meta = {
          description = "Deploy CLI (slices 1-3: registry list + validation, unit rendering, dry-run plans, generation log, status/rollback logic, real systemd activation) for the services deployment layer";
          mainProgram = "deploy";
        };
      }
      ''
        makeWrapper ${python3}/bin/python3 $out/bin/deploy \
          --add-flags ${./deploy.py} \
          --set DEPLOY_REGISTRY ${writeText "deploy-targets.json" (builtins.toJSON jsonDefinitions)}
      '';
}
