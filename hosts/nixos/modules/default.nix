{ inputs, outputs, pkgs, ... }:
let
  system = outputs.system;
  stateVersion = outputs.stateVersion;
  overlays = outputs.overlays.${system};
  # pkgs = outputs.pkgs.${system};
  homeManagerNixosModules = [
    (
      { ... }:
      {
        imports = [
          inputs.home-manager.nixosModules.home-manager
          #vim.nixosModules.${system}
        ];

        home-manager.useUserPackages = true;
        home-manager.useGlobalPkgs = true;
        home-manager.backupFileExtension = "backup";
        # TODO: there is a thing that gets the root flake location
        home-manager.users.user = import ../../../home/user {
          inherit stateVersion;
          pkgs = pkgs;
        };
      }
    )
  ];
  nixosModules =
    homeManagerNixosModules
    ++ [
      #nix-topology.nixosModules.default
      {
        #nixpkgs = pkgs;
        system.stateVersion = "23.11"; # stateVersion;
      }
      # TODO: there is a thing that gets the root flake location
      ../../../hosts/common/modules/configuration.nix # this relies on magic overlays, ? todo: remove overlays from configuration.nix? then add inline let overlay configuration right here below this moduleArrayList.
      #./modules/nixos/cachix.nix
      # TODO: there is a thing that gets the root flake location
      ../../../hosts/nixos/modules/hardware-configuration
    ]
    ++ (import ../../../hosts/common/modules/tailscale-autoconnect.nix)
    ++ [
      # TODO: there is a thing that gets the root flake location
      (import ../../../hosts/common/modules/hyprland.nix) # hyprland = would use flake for hyprland master but had annoying warning about waybar? todo try again. prefer flake. the config for this is setup in homeManager for reasons. could be brought out to nixos module would probably fit better due to my agonies
      #       (import ./modules/nm-applet.nix)
    ]
    ++ [
      (
        { ... }:
        {
          imports = [
            inputs.vim.nixosModules.x86_64-linux # .${system}
          ];
        }
      )
    ];
in
nixosModules
