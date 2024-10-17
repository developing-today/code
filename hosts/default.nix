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
    users = [ "user" "backup" ];
  };
  amd = host {
    profiles = [ "desktop" "printing" ];
    hardware = "framework/13-inch/7040-amd";
    disks = [
      "nvme0n1/4t"
      "tmpfs/root"
    ];
    users = [ "user" "backup" ];
  };
  # apu2c3 = host {
  #   profiles = ["server"];
  #   hardware = "pcengines/apu";
  #   disks = [
  #     "sda1/small"
  #     "tmpfs/root"
  #   ];
  #   users = [ "server" ];
  # };
}
