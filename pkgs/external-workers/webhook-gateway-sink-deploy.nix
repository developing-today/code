# Deploy app for the webhook-gateway-sink Worker — issue #5767,
# external-Worker registration slice. The Worker's source lives in
# dezren39/hatch-personal under workspace/webhook-gateway/sink (not
# this repo), so this app carries the pinned source coordinates and
# stages them at deploy time (sparse checkout of the worker dir at the
# pinned rev — trees only, no full clone). The worker is
# dependency-free, so there is no build and no deploy-time npm install.
# Cloudflare credentials are gathered impurely at deploy time (sops
# account id, wrangler token), so the registry definition names no
# secrets of its own — the phase-1 shape.
{
  writeShellApplication,
  bws,
  git,
  jq,
  nodejs,
  sops,
  wrangler,
}:
writeShellApplication {
  name = "webhook-gateway-sink-deploy";
  runtimeInputs = [
    bws
    git
    jq
    nodejs
    sops
    wrangler
  ];
  text = ''
    export HATCH_PERSONAL_REPO="dezren39/hatch-personal"
    export HATCH_PERSONAL_REV="ae79c332a78a7ba39b6dacd0f30228384d58dfdc"
    export HATCH_WORKER_PATH="workspace/webhook-gateway/sink"
    repo_root=$(git rev-parse --show-toplevel)
    exec bash "$repo_root/pkgs/external-workers/webhook-gateway-sink-deploy.sh" "$@"
  '';
}
