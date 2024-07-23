{ inputs, outputs, ... }:
let
  system = outputs.system;
  stateVersion = outputs.stateVersion;
  overlays = outputs.overlays;

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
        home-manager.users.user = import ../../../home/user/user.nix {
          inherit stateVersion;
          pkgs = import inputs.nixpkgs {
            inherit system;
            overlays = overlays;
            config = {
              allowUnfree = true;
              permittedInsecurePackages = [
                "electron" # le sigh
              ];
            };
          };
        };
      }
    )
  ];
  nixosModules =
    homeManagerNixosModules
    ++ [
      #nix-topology.nixosModules.default
      {
        nixpkgs = {

          overlays = [
            #        zig-overlay.overlays.default
            #alejandra.overlay
            #nix-software-center.overlay
            inputs.vim.overlay.x86_64-linux # .${system}
            #nix-topology.overlays.default
          ]; # overlays; # are overlays needed in home manager? document which/why?

          config = {
            #allowUnfree = true;
            permittedInsecurePackages = [
              "electron" # le sigh
            ];
          };
        };
        system.stateVersion = "23.11"; # stateVersion;
      }
      # TODO: there is a thing that gets the root flake location
      ../../../lib/configuration.nix # this relies on magic overlays, ? todo: remove overlays from configuration.nix? then add inline let overlay configuration right here below this moduleArrayList.
      #sops-nix.nixosModules.sops
      #./modules/sops.nix
      #./modules/nixos/cachix.nix
      # TODO: there is a thing that gets the root flake location
      ../../../hosts/nixos/modules/hardware-configuration
    ]
    ++ [
      # TODO: there is a thing that gets the root flake location
      (import ../../../modules/nixos/hyprland.nix) # hyprland = would use flake for hyprland master but had annoying warning about waybar? todo try again. prefer flake. the config for this is setup in homeManager for reasons. could be brought out to nixos module would probably fit better due to my agonies
      #       (import ./modules/nm-applet.nix)
    ]
    ++ [
      (
        { ... }:
        {
          imports = [
            # home-manager.nixosModules.home-manager
            inputs.vim.nixosModules.x86_64-linux # .${system}
          ];

          #home-manager.useUserPackages = true;
          #home-manager.useGlobalPkgs = true;
          #home-manager.backupFileExtension = "backup";
          #home-manager.users.user = import ./home/user/user.nix;
          #inherit stateVersion;
          #pkgs = pkgsFor.x86_64-linux; # import nixpkgs {
          #inherit system;
          #overlays = overlays;

          #*/
          #config = {
          #allowUnfree = true;
          #permittedInsecurePackages = [
          #  "electron" # le sigh
          #];
          #};
          #};
          # }
        }
      )
    ];
in
nixosModules
