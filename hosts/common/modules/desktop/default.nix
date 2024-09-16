{
  inputs,
  outputs,
  system,
  stateVersion,
  ...
}:
let
  pkgs = import inputs.nixpkgs {
    inherit system;
    config = {
      allowUnfree = true;
      permittedInsecurePackages = [
        "olm-3.2.16"
        "electron"
        "qtwebkit-5.212.0-alpha4"
      ];
    };
    overlays = [
      inputs.vim.overlay.${system}
      inputs.yazi.overlays.default
      inputs.waybar.overlays.default
      # (final: prev: { omnix = inputs.omnix.packages.${system}.default; })
    ];
  };
in
(import ../tailscale-autoconnect.nix)
++ [
  (import ../home.nix { inherit inputs pkgs stateVersion; })
  {
    system.stateVersion = stateVersion;
    nixpkgs.overlays = pkgs.overlays;
  }
  (
    { ... }:
    {
      imports = [ inputs.vim.nixosModules.${system} ];
    }
  )
  ../configuration.nix # this relies on magic overlays, ? todo: remove overlays from configuration.nix? then add inline let overlay configuration right here below this moduleArrayList.
  (import ../hyprland.nix) # hyprland = would use flake for hyprland master but had annoying warning about waybar? todo try again. prefer flake. the config for this is setup in homeManager for reasons. could be brought out to nixos module would probably fit better due to my agonies
]
