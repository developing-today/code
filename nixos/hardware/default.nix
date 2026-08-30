{ pkgs, ... }:
{
  #sound.enable = true; # not needed?
  hardware = {
    brillo.enable = false;
    logitech.wireless.enable = true;
  };
  services = {
    pulseaudio.enable = false;
    udev.packages = with pkgs; [
      platformio-core.udev
      meshtasticd
      openocd
      probe-rs-tools
      stlink
      picotool
      picoprobe-udev-rules
      teensy-udev-rules
      usb-blaster-udev-rules
      qFlipper
      hackrf
      rtl-sdr
    ];
  };
  programs.wireshark = {
    enable = true;
    usbmon.enable = true;
  };
  # Enable Android Debug Bridge (ADB) for phone connectivity
  # programs.adb.enable = true;
}
