{
  #sound.enable = true; # not needed?
  hardware = {
    brillo.enable = false;
    pulseaudio.enable = false;
    logitech.wireless.enable = true;
  };

  # Enable Android Debug Bridge (ADB) for phone connectivity
  programs.adb.enable = true;
}
