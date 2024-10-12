_:
{
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
