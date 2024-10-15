{
  inputs,
  system,
  lib,
  ...
}:
{
  imports = [
    (lib.from-root "hosts/all")
    (lib.from-root "hosts/tailscale-autoconnect")
    (lib.from-root "hosts/home") # users should import home users as-needed
    inputs.vim.nixosModules.${system}
    (lib.from-root "hosts/hyprland") # hyprland = would use flake for hyprland master but had annoying warning about waybar? todo try again. prefer flake. the config for this is setup in homeManager for reasons. could be brought out to nixos module would probably fit better due to my agonies
    (lib.from-root "hosts/sops")
    (lib.from-root "hosts/impermanence")
    (lib.from-root "hosts/boot")
    (lib.from-root "hosts/nixpkgs") # how to get this to home manager?
    (lib.from-root "hosts/system")
    (lib.from-root "hosts/networking")
    (lib.from-root "hosts/i18n")
    (lib.from-root "hosts/time")
    (lib.from-root "hosts/nix")
    (lib.from-root "hosts/users") # users should import home users as-needed
    (lib.from-root "hosts/fonts")
    (lib.from-root "hosts/hardware")
    (lib.from-root "hosts/environment")
    (lib.from-root "hosts/programs")
    (lib.from-root "hosts/services")
    (lib.from-root "hosts/virtualisation") # (sic)
    (lib.from-root "hosts/security")
  ];
}
