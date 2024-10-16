{
  inputs,
  system,
  lib,
  ...
}:
{
  imports = [
    (lib.from-root "hosts/all")
    (lib.from-root "hosts/environment")
    (lib.from-root "hosts/fonts")
    (lib.from-root "hosts/hardware")
    (lib.from-root "hosts/hyprland") # hyprland = would use flake for hyprland master but had annoying warning about waybar? todo try again. prefer flake. the config for this is setup in homeManager for reasons. could be brought out to nixos module would probably fit better due to my agonies
    (lib.from-root "hosts/programs")
    (lib.from-root "hosts/security")
    (lib.from-root "hosts/services")
    (lib.from-root "hosts/virtualisation") # (sic)
    inputs.vim.nixosModules.${system}
  ];
}
