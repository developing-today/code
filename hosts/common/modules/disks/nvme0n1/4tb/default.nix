_:
{
  disko.devices.disk."nvm0n1" ={
    device = "/dev/nvm0n1";
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
            extraArgs = [ "-LNIXBOOT" ];
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
