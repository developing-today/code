{
  writeShellApplication,
  bws,
  git,
  jq,
  nodejs,
  sops,
  wrangler,
  queueTransportAlert,
}:
# code#6369: the deploy app carries the Worker package (package.nix) by
# store path; deploy.mjs stages it, runs wrangler deploy from the staged
# copy, then sets the Worker's secrets from Bitwarden by name.
writeShellApplication {
  name = "queue-transport-alert-deploy";
  runtimeInputs = [
    bws
    git
    jq
    nodejs
    sops
    wrangler
  ];
  text = ''
    export QUEUE_TRANSPORT_ALERT_PKG="${queueTransportAlert}"
    repo_root=$(git rev-parse --show-toplevel)
    exec bash "$repo_root/pkgs/queue-transport-alert/worker/deploy.sh" "$@"
  '';
}
