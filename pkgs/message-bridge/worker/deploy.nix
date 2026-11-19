{
  writeShellApplication,
  bws,
  cf,
  git,
  jq,
  nodejs,
  sops,
  wrangler,
  messageBridge,
}:
# The deploy app carries the Worker package (package.nix) by store
# path; deploy.mjs stages it with node_modules pointing at this cf and
# wrangler (cf resolves cf/config there and delegates the build to
# wrangler; the repo's pkgs/cf and pkgs/wrangler, as queue-transport-alert
# uses), uploads a version with `cf workers versions create`, and
# deploys it with `cf workers deployments create`.
writeShellApplication {
  name = "message-bridge-deploy";
  runtimeInputs = [
    bws
    cf
    git
    jq
    nodejs
    sops
  ];
  text = ''
    export MESSAGE_BRIDGE_PKG="${messageBridge}"
    export MB_CF_MODULE="${cf}/lib/node_modules/cf"
    export MB_WRANGLER_MODULE="${wrangler}/lib/node_modules/wrangler"
    repo_root=$(git rev-parse --show-toplevel)
    exec bash "$repo_root/pkgs/message-bridge/worker/deploy.sh" "$@"
  '';
}
