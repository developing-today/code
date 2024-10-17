{ config, pkgs, ... }:
{
  # Enable CUPS to print documents
  services.printing = {
    enable = true;
    drivers = [ pkgs.gutenprint pkgs.hplip ];
  };
  # Enable avahi for network printer discovery
  services.avahi = {
    enable = true;
    nssmdns = true;
  };
  # Open firewall ports for CUPS
  networking.firewall.allowedTCPPorts = [ 631 ];
  networking.firewall.allowedUDPPorts = [ 631 ];
}
