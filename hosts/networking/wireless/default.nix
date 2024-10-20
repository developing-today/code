{
  config,
  inputs,
  hostName,
  host,
  system,
  stateVersion,
  lib,
  pkgs,
  ...
}:
{
  networking = {
    networkmanager = {
      enable = false;
      unmanaged = [
        "*"
        "except:type:wwan"
        "except:type:gsm"
      ];
    };
    wireless = {
      enable = true;
      # userControlled.enable = true;
      scanOnLowSignal = true;
      fallbackToWPA2 = true;
      # TODO: separate module for secretsFile and template
      #       so that one can enable wireless without adding a network
      #       useful for manual configuration
      secretsFile = config.sops.templates.wireless-secrets.path;
      allowAuxiliaryImperativeNetworks = true; # TODO: can we disable this?
      userControlled = {
        enable = true;
        group = "network";
      };
      # whats extraConfig.update_config=1 do?
      extraConfig = ''
        update_config=1
      '';
    };
  };
  sops.templates.wireless-secrets.content = host.wireless-secrets-template config;
  users.groups.network = { }; # Ensure group exists this would be for users that aren't root or sudoers or doassers or whatever
}
