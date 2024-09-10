{
  boot.loader = {
    systemd-boot.enable = true;
    efi.canTouchEfiVariables = true;
  };
}
# {
#   boot.loader = {
#     systemd-boot = {
#       enable = true;
#       consoleMode = "max";
#     };
#     efi.canTouchEfiVariables = true;
#   };
# }
