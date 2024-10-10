_:
{
  disko.devices.disk."nvme0n1" ={
    device = "/dev/nvme0n1";
    type = "disk";
    content = {
      type = "gpt";
      partitions = {
        ESP = {
          type = "EF00";
          size = "100G"; # 32G? # 4G?
          content = {
            type = "filesystem";
            format = "vfat";
            mountpoint = "/boot";
            extraArgs = [ "-nNIXBOOT" ];
          };
        };
        root = {
          size = "1T";
          content = {
            type = "filesystem";
            format = "ext4";
            mountpoint = "/";
            extraArgs = [ "-LNIXROOT" ];
          };
        };
      };
    };
  };
}
