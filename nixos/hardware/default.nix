{
  #sound.enable = true; # not needed?
  hardware = {
    brillo.enable = false;
    logitech.wireless.enable = true;
  };
  services = {
    pulseaudio.enable = false;
  };
  # Enable Android Debug Bridge (ADB) for phone connectivity
  programs.adb.enable = true;
}
