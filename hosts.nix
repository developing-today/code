let
  key = protocol: alias: builtins.readFile ./keys/${protocol}-${alias}.pub;
  wg = alias: key "wg" alias;
  ssh-host = alias: key "ssh-host" alias;
in
{
  nixos = rec {
    name = "nixos";
    system = "x86_64-linux";
    stateVersion = "23.11";
    wgKey = wg name;
    sshKey = ssh-host name;
  };
  amd = rec {
    name = "amd";
    system = "x86_64-linux";
    stateVersion = "23.11";
    wgKey = wg name;
    sshKey = ssh-host name;
  };
}
