{
  description = "Reproducible Linux netfilter emulation for the C2 holepunch spike";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { nixpkgs, ... }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: builtins.listToAttrs (map (system: {
        name = system;
        value = f system;
      }) systems);
    in {
      apps = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          labPackages = with pkgs; [
            bash cargo coreutils gcc gnugrep gnused iproute2 nftables openssl pkg-config procps python3 rustc sccache util-linux
          ];
          app = pkgs.writeShellScript "c2-holepunch-reproduce" ''
            set -eu
            export PATH="${pkgs.lib.makeBinPath labPackages}"
            spike="$PWD/doc/2026-10-11T03-44-33Z_design_groups_events_delegation/spikes/c2-holepunch"
            if [ ! -f "$spike/lab/reproduce.sh" ]; then
              echo "Run this app from the repository root." >&2
              exit 2
            fi
            exec ${pkgs.bash}/bin/bash "$spike/lab/reproduce.sh" "$@"
          '';
        in {
          c2-lab = {
            type = "app";
            program = "${app}";
          };
        });
    };
}
