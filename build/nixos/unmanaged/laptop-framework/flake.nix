{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable"; # todo figure out how to use this
    flake-utils.url = "github:numtide/flake-utils";
    home = {
      url = "path:./home";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # must type this all out every time
    # because flake inputs are basically static
    # can't make a let var function closure thing around it or whatever
    zig-overlay = {
      url = "github:mitchellh/zig-overlay"; # url = "github:developing-today-forks/zig-overlay/quote-urls";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    neovim-nightly-overlay = {
      url = "github:nix-community/neovim-nightly-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    alejandra = {
      url = "github:kamadorueda/alejandra"; # url = "github:developing-today-forks/alejandra/quote-urls";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # nix-rice = https://github.com/bertof/nix-rice # todo fork and rename this garbage
  };

  outputs = {
    self,
    nixpkgs,
    zig-overlay,
    neovim-nightly-overlay,
    flake-utils,
    home,
    alejandra,
    ...
  }: let
    stateVersion = "23.11";
    #     prettierTomlOverlay = pkgs: super: {
    #       prettierToml = pkgs.writeShellScriptBin "prettier" ''
    #         ${pkgs.nodePackages.prettier}/bin/prettier --plugin prettier-plugin-toml \
    #         "$@"
    #       '';
    #     };
    overlays = [
      zig-overlay.overlays.default
      neovim-nightly-overlay.overlay
      alejandra.overlay
      #       prettierTomlOverlay
    ];
    systemNixOsModules = [
      {
        nixpkgs = {
          overlays = overlays; # are overlays needed in home manager? document which/why?
          config.allowUnfree = true;
        };
        system.stateVersion = stateVersion;
      }
      ./configuration.nix # this relies on magic overlays, ? todo: remove overlays from configuration.nix? then add inline let overlay configuration right here below this moduleArrayList.
    ];
    # overlayNixOsModules = ?
    hyprlandNixOsModules = [
      (import ./programs/hyprland/enable.nix) # hyprland = would use flake for hyprland master but had annoying warning about waybar? todo try again. prefer flake. the config for this is setup in homeManager for reasons. could be brought out to nixos module would probably fit better due to my agonies
    ];
    system = "x86_64-linux";
    pkgs = import nixpkgs {
      system = system;
      overlays = overlays;
    };

    homeManagerNixOsModules = home.homeManagerNixOsModules stateVersion;
  in {
    nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
      inherit system;
      modules =
        systemNixOsModules
        ++ hyprlandNixOsModules
        ++ homeManagerNixOsModules;
    };
  };
}
