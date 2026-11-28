{
  inputs,
  hostName,
  lib,
  ...
}:
{
  imports = [
    inputs.server.nixosModules.microvm
    inputs.server.nixosModules.server
  ];
  server = {
    network = {
      inherit hostName;
      vlans = { };
      dns = "9.9.9.9";

      vlans = {
        management = {
          # The name of the VLAN interface
          id = 90; # The VLAN ID
          ip = "10.0.90.2"; # The IP for the host
          prefix = 24; # The prefix of the subnet (10.0.90.2/24)
          gateway = "10.0.90.1"; # The default gateway for the host and any container/VM that gets attached to this VLAN
          parentInterface = "eth0"; # The parent interface of the VLAN
        };

        my-vlan-10 = {
          # The name of the VLAN interface
          id = 10; # The VLAN ID
          prefix = 24; # The prefix of the subnet
          gateway = "10.0.10.1"; # The default gateway for any container/VM that gets attached to this VLAN
          parentInterface = "eth0"; # The parent interface of the VLAN
        };
      };
    };
    vms = {
      some-vm = {
        vcpus = 1; # The number of vCPUs
        ramMb = 512; # The amount of RAM in MB
        # volumes = [{                # The MicroVM volumes to create for this VM (Docs: https://astro.github.io/microvm.nix/shares.html)
        #   image = "data.img";       # The file name (Stored in `/var/lib/microvms/<vm-name>/`)
        #   mountPoint = "/data";     # The mount point inside the VM
        #   size = 1024;              # The size of the volume in MB
        # }];

        ip = "10.0.10.20"; # The IP of the VM
        vlan = "my-vlan-10"; # The VLAN to which this VM will get attached

        dns = "9.9.9.9"; # Override the DNS for this VM
        # gateway = "10.0.10.100";    # Override the default gateway for this VM
        extraConfiguration = {
          virtualisation = {
            docker.enable = lib.mkForce true;
          };
        };
      };
    };
  };
}
