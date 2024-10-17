{ config, pkgs, ... }:

{
  # Enable CUPS to print documents
  services.printing = {
    enable = true;
    drivers = [ pkgs.gutenprint pkgs.hplip pkgs.postscript-lexmark ];
  };

  # Enable avahi for network printer discovery
  services.avahi = {
    enable = true;
    nssmdns = true;
  };

  # Open firewall ports for CUPS
  networking.firewall.allowedTCPPorts = [ 631 ];
  networking.firewall.allowedUDPPorts = [ 631 ];

  # Add support for Dell printers
  hardware.printers = {
    ensurePrinters = [
      {
        name = "Dell_1815dn";
        location = "Office";
        deviceUri = "usb://Dell/1815dn?serial=SERIAL_NUMBER";
        model = "drv:///sample.drv/generpcl.ppd";
        ppdOptions = {
          PageSize = "A4";
        };
      }
    ];
    ensureDefaultPrinter = "Dell_1815dn";
  };
}
