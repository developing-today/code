{ lib, ... }:
{
  imports = [ ];
  disabledModules = [ "./helloWorld.nix" ];
  options = {
    # optionName = mkEnableOption "this cool module";
    optionName = lib.mkOption {
      type = lib.types.bool;
      default = false;
      example = true;
      description = "Whether to enable this cool module.";
    };
  };
  config = { };
}
