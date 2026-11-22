{
  writeShellApplication,
  git,
  nodejs,
  cf,
  wrangler,
  coldStoreRehydratorBundle,
}:
# Carries the flake-built Worker bundle and the repository-pinned cf and
# Wrangler build bridge. Upload and promotion are separate; upload creates
# a preview version only, and the promotion command targets an explicit id.
writeShellApplication {
  name = "cold-store-rehydrator-deploy";
  runtimeInputs = [
    git
    nodejs
    cf
    wrangler
  ];
  text = ''
    export COLD_STORE_REHYDRATOR_BUNDLE="${coldStoreRehydratorBundle}/worker.js"
    export CSR_CF_MODULE="${cf}/lib/node_modules/cf"
    export CSR_WRANGLER_MODULE="${wrangler}/lib/node_modules/wrangler"
    repo_root="$PWD"
    while [[ ! -e "$repo_root/.git" && "$repo_root" != / ]]; do
      repo_root="''${repo_root%/*}"
    done
    if [[ ! -e "$repo_root/.git" ]]; then
      echo "cold-store-rehydrator-deploy must run from inside the repository" >&2
      exit 2
    fi
    CSR_CHECK_ARGS_ONLY=1 node "$repo_root/pkgs/cold-store-reaper/worker/deploy.mjs" "$@"
    exec bash "$repo_root/pkgs/cold-store-reaper/worker/deploy.sh" "$@"
  '';
}
