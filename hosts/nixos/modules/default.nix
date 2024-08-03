{ inputs, outputs, ... }:
let
  system = outputs.system;
  stateVersion = outputs.stateVersion;
  overlays = outputs.overlays.${system};
  pkgs = outputs.pkgs.${system};
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
      ../../../lib/configuration.nix # this relies on magic overlays, ? todo: remove overlays from configuration.nix? then add inline let overlay configuration right here below this moduleArrayList.
      #sops-nix.nixosModules.sops
      #./modules/sops.nix
      #./modules/nixos/cachix.nix
      # TODO: there is a thing that gets the root flake location
      ../../../hosts/nixos/modules/hardware-configuration
    ]
    ++ [
      (import ../../../hosts/common/modules/tailscale-autoconnect.nix)
      (
        { config, ... }:
        {
          services.tailscaleAutoconnect = {
            enable = true;
            authkeyFile = config.sops.secrets.tailscale_key.path;
            loginServer = "https://login.tailscale.com";
            # default login server is controlplane, unsure why we are changing it.
            #exitNode = "some-node-id";
            #exitNodeAllowLanAccess = true;
          };
          # default
          sops.secrets.tailscale_key = { };
          #  sopsFile = ../../../lib/config.enc/common/secrets.yaml;
          #};
        }
      )
      # TODO: there is a thing that gets the root flake location
      (import ../../../hosts/common/modules/hyprland.nix) # hyprland = would use flake for hyprland master but had annoying warning about waybar? todo try again. prefer flake. the config for this is setup in homeManager for reasons. could be brought out to nixos module would probably fit better due to my agonies
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
          # "qtwebkit-5.212.0-alpha4" # ???
          #];
          #};
          #};
          # }
        }
      )
    ];
in
nixosModules
