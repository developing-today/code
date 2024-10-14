{
  config,
  inputs,
  hostName,
  host,
  system,
  stateVersion,
  lib,
  ...
}:{
  # sops secret
  # sops.secrets.github-token-root = {
  # sops.secrets.github-token-user = {
  #   path = ""
  #   sopsFile = lib.from-root "secrets/sops/common/tailscale.yaml";
  # };
}
