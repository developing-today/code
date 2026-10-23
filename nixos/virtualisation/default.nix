{
  virtualisation = {
    libvirtd.enable = true;
    libvirtd.qemu.runAsRoot = true;
    libvirtd.allowedBridges = [
      "virbr0"
      "br0"
    ];
    docker.enable = true;

    # Option 1: Port forwarding
    # forwardPorts = [
    #   { from = "host"; host.port = 2222; guest.port = 22; }  # SSH
    #   { from = "host"; host.port = 8080; guest.port = 80; }  # HTTP example
    # ];

    # Option 2: Virtual network interface (recommended for ping)
    # vlans = [ 1 ];
    # networks = {
    #   "1" = {
    #     # name = "vnet"; # Optional: custom interface name
    #     addressPrefix = "192.168.100";  # Creates 192.168.100.0/24 network
    #     # localAddress = "192.168.100.10";  # Optional: specific guest IP
    #     # hostAddress = "192.168.100.1";    # Optional: specific host IP
    #     vlan = 1;
    #   };
    # };
  };
}
