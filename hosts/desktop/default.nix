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
      (from-root "hosts/all")
      (from-root "hosts/environment")
      (from-root "hosts/fonts")
      (from-root "hosts/virtualisation")
      (from-root "hosts/hardware")
      (from-root "hosts/hyprland") # hyprland = would use flake for hyprland master but had annoying warning about waybar? todo try again. prefer flake. the config for this is setup in homeManager for reasons. could be brought out to nixos module would probably fit better due to my agonies
      (from-root "hosts/programs")
      (from-root "hosts/security")
      (from-root "hosts/services")
      (from-root "hosts/virtualisation") # (sic)
    ];
    # TODO: fix
    # ++ [ inputs.vim.nixosModules.${system} ];
}
