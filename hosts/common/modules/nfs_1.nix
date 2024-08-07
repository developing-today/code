{
  boot.supportedFilesystems = [ "nfs" ];

  services.rpcbind.enable = true;

  networking.firewall =
    let
      ports = [
        2049
        4000
        4001
        4002
      ];
    in
    {
      allowedTCPPorts = ports;
      allowedUDPPorts = ports;
    };
}
