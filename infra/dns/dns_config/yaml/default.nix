let
  pkgs = import <nixpkgs> {};
  DNSConfig = import ./.. { inherit (pkgs) lib; };
in
pkgs.runCommand "dns_config.yaml" { buildInputs = [ pkgs.yq-go ]; } ''
  echo '${builtins.toJSON DNSConfig.DNSConfig}' | yq eval -P > $out
''
