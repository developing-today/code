{
  inputs,
  system,
  lib,
  ...
}:
{
  imports =
    with lib;
    [
      (from-root "nixos/all")
      (from-root "nixos/environment")
      (from-root "nixos/fonts")
      (from-root "nixos/virtualisation")
      (from-root "nixos/hardware")
      (from-root "nixos/hyprland") # hyprland = would use flake for hyprland master but had annoying warning about waybar? todo try again. prefer flake. the config for this is setup in homeManager for reasons. could be brought out to nixos module would probably fit better due to my agonies
      (from-root "nixos/programs")
      (from-root "nixos/security")
      (from-root "nixos/services")
      (from-root "nixos/virtualisation") # (sic)
    ]
    ++ [ inputs.self.nixosModules.${system} ];
}
