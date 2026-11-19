{
  writeShellApplication,
  bws,
  git,
  jq,
  nodejs,
  sops,
  wrangler,
  messageBridge,
}:
# The deploy app carries the Worker package (package.nix) by store
# path; deploy.mjs stages it, runs wrangler deploy from the staged copy,
# then sets the Worker's access token from Bitwarden by name.
writeShellApplication {
  name = "message-bridge-deploy";
  runtimeInputs = [
    bws
    git
    jq
    nodejs
    sops
    wrangler
  ];
  text = ''
    export MESSAGE_BRIDGE_PKG="${messageBridge}"
    repo_root=$(git rev-parse --show-toplevel)
    exec bash "$repo_root/pkgs/message-bridge/worker/deploy.sh" "$@"
  '';
}
