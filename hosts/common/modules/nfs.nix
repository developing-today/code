{
  services.nfs = {
    server = {
      enable = true;
      exports = ''
        /shared *(rw,async,wdelay,root_squash,no_subtree_check)
      '';
    };
  };
}
# {
#   boot.supportedFilesystems = [ "nfs" ];

#   services.rpcbind.enable = true;

#   networking.firewall =
#     let
#       ports = [
#         2049
#         4000
#         4001
#         4002
#       ];
#     in
#     {
#       allowedTCPPorts = ports;
#       allowedUDPPorts = ports;
#     };
# }
