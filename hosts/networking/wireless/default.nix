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
  imports = [
    (lib.from-root "hosts/networking/wireless/us-wi-1")
  ];
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
      secretsFile = config.sops.templates.wireless.path;
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
  sops.templates."wireless".content = "${config.sops.placeholder."wireless_us-wi-1"}";
  sops.templates.test_template.content = "test 123";
  # # Ensure group exists
  # this would be for users that aren't root or sudoers or doassers or whatever
  users.groups.network = { };
}
