{
  pkgs
}:
{
  isNormalUser = true;
  description = "user";
  extraGroups = [
    "trusted-users"
    "networkmanager"
    "wheel"
    "docker"
    "video"
    "kvm"
    "beep"
  ];
  packages = with pkgs; [
    firefox
    kate
  ];
}
