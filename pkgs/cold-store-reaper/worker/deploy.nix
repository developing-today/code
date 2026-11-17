{
  writeShellApplication,
  bws,
  git,
  jq,
  nodejs,
  sops,
  wrangler,
  coldStoreRehydratorBundle,
}:
# Issue #5767, phase 2: the deploy app carries the flake-built Worker
# bundle (package.nix) by store path; deploy.mjs stages it with
# wrangler.toml and uploads it through the versions flow. The manual
# REST PUT of the committed dist/ bundle is retired.
writeShellApplication {
  name = "cold-store-rehydrator-deploy";
  runtimeInputs = [
    bws
    git
    jq
    nodejs
    sops
    wrangler
  ];
  text = ''
    export COLD_STORE_REHYDRATOR_BUNDLE="${coldStoreRehydratorBundle}/worker.js"
    repo_root=$(git rev-parse --show-toplevel)
    exec bash "$repo_root/pkgs/cold-store-reaper/worker/deploy.sh" "$@"
  '';
}
