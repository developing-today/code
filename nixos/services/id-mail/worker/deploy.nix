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
  name = "id-mail-deploy";
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
    exec bash "$repo_root/nixos/services/id-mail/worker/deploy.sh" "$@"
  '';
}
