# TODO: consider if there's a better way to configure this, possibly a function that generates the module?
#       or maybe add a few 'standard' tmpfs disk sizes?
{ inputs, lib, ... }:
{
  imports = [
    (lib.from-root "hosts/disks")
  ];
  disko.devices = {
    nodev."/" = {
      fsType = "tmpfs";
      mountOptions = [
        "defaults"
        "size=2G" # 4 8 16 ?
        "mode=755"
      ];
    };
  };
}
