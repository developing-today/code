{ self ? builtins.getFlake "self" }:
let
  getBasePath = if builtins.hasAttr "outPath" self
    then self.outPath  # Flake-based setup
    else builtins.toString ../.; # Traditional Nix setup, resolve to project root
  resolvePath = path: "${getBasePath}/${path}";
  public-key = protocol: alias: builtins.readFile (resolvePath "keys/${protocol}-${alias}.pub");
  group-key = alias: public-key "ssh-group" alias;
  host-key = alias: public-key "ssh-host" alias;
  user-key = alias: public-key "ssh-user" alias;
in {
  inherit public-key group-key host-key user-key;
}
