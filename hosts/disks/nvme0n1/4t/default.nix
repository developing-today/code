# TODO: add 10% unallocated space
# TODO: consider if there's a better way to configure this, possibly a function that generates the module?
{
  disko.devices.disk."nvme0n1" = {
    device = "/dev/nvme0n1";
    type = "disk";
    content = {
      type = "gpt";
      partitions = {
        ESP = {
          # TODO: use grub and move kernels to /persistence/boot/efi or something
          size = "100G"; # 32G? # 4G?
          type = "EF00";
          content = {
            type = "filesystem";
            format = "vfat";
            mountpoint = "/boot";
            mountOptions = [ "umask=0077" ];
            # extraArgs = [ "-nNIXBOOT" ];
          };
        };
        nix = {
          end = "-200G"; # 200G swap for 96G ram
          content = {
            type = "filesystem";
            format = "ext4";
            mountpoint = "/nix";
            # extraArgs = [ "-LNIXROOT" ];
          };
        };
        swap = {
          size = "100%";
          content = {
            type = "swap";
            discardPolicy = "both";
            resumeDevice = true; # resume from hiberation from this device
          };
        };
      };
    };
  };
}
