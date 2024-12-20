{
  programs = {
    # bash.enable = true;
    fish.enable = true;
    zsh.enable = true;
    git = {
      enable = true;
      config = {
        safe.directory = [ "*" ];
      };
    };
    partition-manager.enable = true;
    steam = {
      enable = true;
      remotePlay.openFirewall = true; # Open ports in the firewall for Steam Remote Play
      dedicatedServer.openFirewall = true; # Open ports in the firewall for Source Dedicated Server
    };
    kdeconnect = {
      enable = true;
    };
  };
}
