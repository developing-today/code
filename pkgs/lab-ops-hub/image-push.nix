{
  writeShellApplication,
  bws,
  crane,
  git,
  nix,
  nodejs,
  sops,
  labOpsHub,
}:
# Operator-only crawler image build and push. Worker code is released by
# the separate GHA cf Workers Versions flow.
writeShellApplication {
  name = "lab-ops-hub-image-push";
  runtimeInputs = [
    bws
    crane
    git
    nix
    nodejs
    sops
  ];
  text = ''
    LOH_CHECK_ARGS_ONLY=1 node ${./image-push.mjs} "$@"
    export LAB_OPS_HUB_PKG="${labOpsHub}"
    repo_root=$(git rev-parse --show-toplevel)
    exec bash "$repo_root/pkgs/lab-ops-hub/image-push.sh" "$@"
  '';
}
