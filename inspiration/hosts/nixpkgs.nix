{ outputs, ... }:
{
  nixpkgs = {
    overlays = builtins.attrValues outputs.overlays;
    config = {
      allowBroken = true;
      allowUnfree = true;
      allowUnfreePredicate = _: true;
    };
  };
}
