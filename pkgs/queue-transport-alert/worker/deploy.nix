{
  writeShellApplication,
  bash,
  bws,
  cf,
  git,
  nodejs,
  sops,
  wrangler,
  queueTransportAlert,
}:
# code#6369: the deploy app carries the Worker package (package.nix) by
# store path; deploy.mjs stages it and deploys through the cf versions
# flow. cf/config and wrangler (cf's build bridge for a plain-JS Worker)
# are linked into the staged node_modules from the store, so a deploy
# needs no npm install.
writeShellApplication {
  name = "queue-transport-alert-deploy";
  runtimeInputs = [
    bash
    bws
    cf
    git
    nodejs
    sops
  ];
  text = ''
    export QUEUE_TRANSPORT_ALERT_PKG="${queueTransportAlert}"
    export QTA_CF_MODULE="${cf}/lib/node_modules/cf"
    export QTA_WRANGLER_MODULE="${wrangler}/lib/node_modules/wrangler"
    repo_root=$(git rev-parse --show-toplevel)
    exec bash "$repo_root/pkgs/queue-transport-alert/worker/deploy.sh" "$@"
  '';
}
