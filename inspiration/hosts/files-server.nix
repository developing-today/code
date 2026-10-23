{
  services.nginx.virtualHosts = {
    "merope.m7.rs" = {
      forceSSL = true;
      enableACME = true;
      locations."/".root = "/srv/files";
    };
  };
}
# let
#   files = {
#     forceSSL = true;
#     enableACME = true;
#     locations."/".root = "/srv/files";
#   };
# in
# {
#   services.nginx.virtualHosts = {
#     "files.m7.rs" = files;
#     "f.m7.rs" = files;
#   };
# }
