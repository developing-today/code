{
  writeShellApplication,
  bws,
  crane,
  git,
  jq,
  nix,
  nodejs,
  sops,
  labOpsHub,
}:
# Issue #5767, phase 2: one `deploy lab-ops-hub` — the deploy app
# carries the flake package (package.nix: Worker tree + lockfile-
# pinned deps incl. wrangler) by store path; deploy.mjs stages it,
# builds + pushes the crawler container image (container-image.nix),
# and runs the pinned wrangler deploy from the staged copy.
writeShellApplication {
  name = "lab-ops-hub-deploy";
  runtimeInputs = [
    bws
    crane
    git
    jq
    nix
    nodejs
    sops
  ];
  text = ''
    export LAB_OPS_HUB_PKG="${labOpsHub}"
    repo_root=$(git rev-parse --show-toplevel)
    exec bash "$repo_root/pkgs/lab-ops-hub/deploy.sh" "$@"
  '';
}
