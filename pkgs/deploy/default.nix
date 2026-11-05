# The deploy registry + CLI — issue #5766, slice 1 (registry + list).
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
    fleet = fleetPkgs.fleet;
    mcpx = callPackage ../mcpx/package.nix { };
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
          description = "Deploy CLI (slice 1: registry list + definition validation) for the services deployment layer";
          mainProgram = "deploy";
        };
      }
      ''
        makeWrapper ${python3}/bin/python3 $out/bin/deploy \
          --add-flags ${./deploy.py} \
          --set DEPLOY_REGISTRY ${writeText "deploy-targets.json" (builtins.toJSON jsonDefinitions)}
      '';
}
