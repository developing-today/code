{ config, pkgs, ... }:
{
  imports = [
  ];
  disabledModules = [ "./helloWorld.nix" ];
  options = {
    # optionName = mkEnableOption "this cool module";
    optionName = mkOption {
    type = lib.types.bool;
    default = false;
    example = true;
    description = "Whether to enable this cool module.";
    }
  };
  config = {
  };
}