let
  key = protocol: alias: builtins.readFile ./keys/${protocol}-${alias}.pub;
  ssh-user = alias: key "ssh-user" alias;
in
{
  user = rec {
    enable = true;
    name = "user"
    uid = 1000;
    groups = [ "wheel" ];
    keys = [ (ssh-user name) ];
    email = "dsp@developing-today.com";
    aliases = [ "drewrypope@gmail.com" ];
  }
}
