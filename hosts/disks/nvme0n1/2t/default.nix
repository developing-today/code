# TODO: add 10% unallocated space
# TODO: consider if there's a better way to configure this, possibly a function that generates the module?
{ inputs, lib, ... }:
{
  imports = [ (lib.from-root "hosts/disks") ];
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
          end = "-128G"; # 128G swap for 64G ram
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
