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
  name = "developing-today-backup-deploy";
  runtimeInputs = [
    bws
    git
    jq
    nodejs
    sops
    wrangler
  ];
  text = ''
    repo_root=$(git rev-parse --show-toplevel)
    exec bash "$repo_root/infra/github-backup/worker/deploy.sh" "$@"
  '';
}
