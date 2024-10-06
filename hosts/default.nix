inputs:
let
host = inputs.self.lib.nixos-host-configuration;
in {
  nixos = host {
    profiles = "desktop";
    hardware = "framework/13-inch/12th-gen-intel";
    disks = "nvme0n1/2tb";
  };
  amd = host {
    profiles = "desktop";
    hardware = "framework/13-inch/7040-amd";
    disks = "nvme0n1/4tb";
  };
}
