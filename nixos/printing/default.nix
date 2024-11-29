{ lib, pkgs, ... }:
{
  # Make the PPD file available in the Nix store
  nixpkgs.overlays = [
    (final: prev: {
      dell1815dnPPD = final.runCommand "dell1815dn-ppd" { } ''
        mkdir -p $out/share/cups/model
        cp ${(lib.from-root "nixos/printing/mfp1815ps.ppd")} $out/share/cups/model/mfp1815ps.ppd
      '';
    })
  ];

  # Printer configuration
  services.printing = {
    enable = true;
    drivers = [
      pkgs.gutenprint
      pkgs.hplip
      pkgs.dell1815dnPPD
    ];
  };

  hardware.printers = {
    ensurePrinters = [
      {
        name = "Dell_1815dn";
        location = "Office";
        # deviceUri = "usb://Dell/1815dn?serial=SERIAL_NUMBER";
        # direct usb://Dell/Laser%20MFP%201815?serial=CQQLCD1.........&interface=1
        # foomatic-db
        # https://aur.archlinux.org/packages/dell-unified-driver
        # https://www.bchemnet.com/suldr/supported.html
        # openprinting
        # https://ubuntuforums.org/archive/index.php/t-768745.html
        deviceUri = "usb://Dell/Laser%20MFP%201815?serial=CQQLCD1&interface=1";
        model = "mfp1815ps.ppd";
        ppdOptions = {
          PageSize = "A4";
        };
      }
    ];
    ensureDefaultPrinter = "Dell_1815dn";
  };

  # CUPS and firewall configuration
  services.avahi = {
    enable = true;
    nssmdns = true;
  };
  networking.firewall.allowedTCPPorts = [ 631 ];
  networking.firewall.allowedUDPPorts = [ 631 ];
}
