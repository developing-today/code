inputs:
let
  host = inputs.self.lib.nixos-host-configuration;
in
{
  nixos = host {
    profiles = "desktop";
    hardware = "framework/13-inch/12th-gen-intel";
    disks = [
      "nvme0n1/2t"
      "tmpfs/root"
    ];
    users = [
      "user"
      "backup"
    ];
    wireless = "us-wi-1";
  };
  amd = host {
    profiles = [
      "desktop"
      # "printing"
      "services/flatpak"
      # "microvm"
      # "server"
      "networking/dhcp-nat"
    ];
    hardware = "framework/13-inch/7040-amd";
    disks = [
      "nvme0n1/4t"
      "tmpfs/root"
    ];
    users = [
      "user"
      "backup"
    ];
    wireless = [
      "us-wi-1"
      "global-mobile-1"
      "us-wi-2"
      "us-global-1"
      "us-global-2"
    ];
  };
  amd-server = host {
    # profiles = [ "server" ];
    profiles = [ "all" ];
    # hardware = "generic/amd";
    # disks = [
    #   # "nvme0n1/1t"
    #   # "tmpfs/root"
    # ];
    users = [ "user-for-vm" ];
    # wireless = "us-wi-1";
  };
}
# apu2c3 = host {
#   profiles = ["server"];
#   hardware = "pcengines/apu";
#   disks = [
#     "sda1/small"
#     "tmpfs/root"
#   ];
#   users = [ "server" ];
# };
# apu2c4
# apu2c4-with-wifi
# apu2c3-with-modem
# apu2c3-with-wifi
# apu2c3-with-modem-and-wifi
# pi0
# pi2
# pi3
# pi4
# pi5
# fire3
# amd-server
# intel-server
# generic profile to connect/mount data disks
# generic script/process to apply a different config and use that for auto-upgrade going forward
