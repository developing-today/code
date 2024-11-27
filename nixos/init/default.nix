{ host, lib, ... }:
{
  imports =
    with lib;
    lists.flatten [
      (ensure-list host.modules)
      (ensure-list host.imports)

      (make-hardware host.hardware)
      (ensure-list host.hardware-modules)
      (ensure-list host.hardware-imports)

      (make-profiles host.profiles)
      (ensure-list host.profile-modules)
      (ensure-list host.profile-imports)

      (make-disks host.disks)
      (ensure-list host.disk-modules)
      (ensure-list host.disk-imports)

      (make-wireless host.wireless)
      (ensure-list host.wireless-modules)
      (ensure-list host.wireless-imports)

      (make-users host.users)
      (ensure-list host.user-modules)
      (ensure-list host.user-imports)

      # (ensure-list host.darwin-profiles) # make-darwin-profiles
      (ensure-list host.darwin-profile-modules)
      (ensure-list host.darwin-profile-imports)
      (ensure-list host.darwin-modules)
      (ensure-list host.darwin-imports)
    ];
}
/*
  # (make-darwin-modules host.darwin-profiles)
  # networking # TODO: make this work
  # TODO: make generic array function and use that, maybe prefix one is enough?
  # TODO: fn to allow optionals for the auto-list below, removed before import
  from-root "nixos/abstract" # maybe don't import all, just ones needed as needed?
  from-root "nixos/hardware-configuration/${hostName}"
  from-root "nixos/{host.type}"
  from-root "nixos/{host.type}/{hostName}"
  from-root "nixos/{host.type}/{hostName}/{profile}" for profile in host.profiles
  from-root "nixos/{host.type}/{profile}" for profile in host.profiles
  from-root "nixos/{host.type}/{profile}/{hostName}" for profile in host.profiles
  from-root "nixos/{hostName}"
  from-root "nixos/{hostName}/{host.type}"
  from-root "nixos/{hostName}/{host.type}/{profile}" for profile in host.profiles
  from-root "nixos/{hostName}/{profile}" for profile in host.profiles
  from-root "nixos/{hostName}/{profile}/{host.type}" for profile in host.profiles
  from-root "nixos/{profile}" for profile in host.profiles
  from-root "nixos/{profile}/{host.type}" for profile in host.profiles
  from-root "nixos/{profile}/{hostName}" for profile in host.profiles
  from-root "nixos/{profile}/{host.type}/{hostName}" for profile in host.profiles
  from-root "nixos/{profile}/{hostName}/{host.type}" for profile in host.profiles
*/
