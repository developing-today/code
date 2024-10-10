_:
{
fileSystems."/" = {
  device = "/dev/disk/by-label/NIXROOT";
  fsType = "ext4";
};
fileSystems."/boot" = {
  device = "/dev/disk/by-label/NIXBOOT";
  fsType = "vfat";
  options = [
    "fmask=0022"
    "dmask=0022"
  ];
};
  # disko.devices.disk."nvm0n1" ={
  #   device = "/dev/nvm0n1";
  #   type = "disk";
  #   content = {
  #     type = "gpt";
  #     partitions = {
  #       ESP = {
  #         type = "EF00";
  #         size = "100G";
  #         content = {
  #           type = "filesystem";
  #           format = "vfat";
  #           mountpoint = "/boot";
  #           extraArgs = [ "-LNIXBOOT" ];
  #         };
  #       };
  #       root = {
  #         size = "1T";
  #         content = {
  #           type = "filesystem";
  #           format = "ext4";
  #           mountpoint = "/";
  #           extraArgs = [ "-LNIXROOT" ];
  #         };
  #       };
  #     };
  #   };
  # };
}
