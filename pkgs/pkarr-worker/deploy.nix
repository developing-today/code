{
  writeShellApplication,
  bws,
  cf,
  git,
  nodejs,
  sops,
}:
writeShellApplication {
  name = "pkarr-deploy";
  runtimeInputs = [
    bws
    cf
    git
    nodejs
    sops
  ];
  text = ''
    repo_root=$(git rev-parse --show-toplevel)
    exec bash "$repo_root/pkgs/pkarr-worker/deploy.sh" "$@"
  '';
}
