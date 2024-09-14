{ inputs, outputs, pkgs, ... }:
let
  system = outputs.system;
  stateVersion = outputs.stateVersion;
  overlays = outputs.overlays.${system};
in
(import ../tailscale-autoconnect.nix)
++ [
  (import ../home.nix { inherit inputs pkgs stateVersion; })
  {
    system.stateVersion = "23.11"; # stateVersion;
  }
  (
    { ... }:
    {
      imports = [
        inputs.vim.nixosModules.x86_64-linux # .${system}
      ];
    }
  )
  ../configuration.nix # this relies on magic overlays, ? todo: remove overlays from configuration.nix? then add inline let overlay configuration right here below this moduleArrayList.
  (import ../hyprland.nix) # hyprland = would use flake for hyprland master but had annoying warning about waybar? todo try again. prefer flake. the config for this is setup in homeManager for reasons. could be brought out to nixos module would probably fit better due to my agonies
  ]
