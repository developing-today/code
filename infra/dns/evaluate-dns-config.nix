let
  pkgs = import <nixpkgs> {};
  dnsConfig = import ./dns-config.nix { inherit (pkgs) lib; };
in
pkgs.runCommand "dns-config.yaml" { buildInputs = [ pkgs.yq-go ]; } ''
  echo '${builtins.toJSON dnsConfig.dnsConfig}' | yq eval -P > $out
''
