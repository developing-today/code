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
      secretsFile = config.sops.secrets.wireless.path;
      networks."I win again, Lews Therin.".pskRaw = "ext:iwinagainlewstherin";
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
  sops.secrets."wireless" = {
    # TODO: us-wi-1 module in hosts/networking/wireless/us-wi-1, make-wireless if wireless is not []
    sopsFile = lib.from-root "secrets/sops/common/networking/wireless/us-wi-1.yaml";
  };
  # # Ensure group exists
  # this would be for users that aren't root or sudoers or doassers or whatever
  users.groups.network = { };
}
