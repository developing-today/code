{
  lib,
  ...
}:
{
  sops.secrets.github-token-root = {
    path = "/home/user/auth";
    sopsFile = lib.from-root "secrets/sops/groups/admin/github.yaml";
  };
  sops.secrets.github-token-user = {
    path = "/root/auth";
    sopsFile = lib.from-root "secrets/sops/groups/admin/github.yaml";
  };
}
