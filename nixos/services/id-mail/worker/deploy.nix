{
  writeShellApplication,
  git,
  nodejs,
  sops,
}:
writeShellApplication {
  name = "id-mail-deploy";
  runtimeInputs = [
    git
    nodejs
    sops
  ];
  text = ''
    repo_root=$(git rev-parse --show-toplevel)
    exec bash "$repo_root/nixos/services/id-mail/worker/deploy.sh" "$@"
  '';
}
