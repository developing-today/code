{ inputs, ... }:
{
  imports = [ inputs.home-manager.nixosModules.home-manager ];
  home-manager.useUserPackages = true;
  home-manager.useGlobalPkgs = true;
  home-manager.backupFileExtension = "backup";
}
