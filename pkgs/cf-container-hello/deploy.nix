{
  writeShellApplication,
  bws,
  git,
  jq,
  nodejs,
  sops,
  wrangler,
  cfContainerHello,
}:
# Issue #5767, phase 2: the deploy app carries the flake package
# (package.nix: Worker tree + lockfile-pinned deps) by store path;
# deploy.mjs stages it and runs wrangler deploy from the staged copy.
# The container images in wrangler.jsonc are pre-pushed registry
# images, so the deploy needs no Docker daemon.
writeShellApplication {
  name = "cf-container-hello-deploy";
  runtimeInputs = [
    bws
    git
    jq
    nodejs
    sops
    wrangler
  ];
  text = ''
    export CF_CONTAINER_HELLO_PKG="${cfContainerHello}"
    repo_root=$(git rev-parse --show-toplevel)
    exec bash "$repo_root/pkgs/cf-container-hello/deploy.sh" "$@"
  '';
}
