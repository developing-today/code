{
  lib,
  ...
}:
{
  imports = [
    (lib.from-root "nixos/users")
    (lib.from-root "home/root")
  ];
}
