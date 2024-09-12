{ inputs, pkgs, stateVersion }:
{
  imports = [
    inputs.home-manager.nixosModules.home-manager
    #vim.nixosModules.${system}
  ];

  home-manager.useUserPackages = true;
  home-manager.useGlobalPkgs = true;
  home-manager.backupFileExtension = "backup";
  home-manager.users.user = import ../../../home/user {
    inherit stateVersion pkgs;
  };
}
