{
  config,
  inputs,
  lib,
  pkgs,
  ...
}:
{
  imports = [
    (lib.from-root "nixos/nix/settings")
    # inputs.determinate.nixosModules.default
  ];
  nix = {
    # settings.flake-registry = ""; # https://github.com/NixOS/nix/issues/8953#issuecomment-1919310666
    registry = lib.mkForce (lib.mapAttrs (_: value: { flake = value; }) inputs); # This will add each flake input as a registry. To make nix3 commands consistent with your flake
    nixPath = lib.mapAttrsToList (key: value: "${key}=${value.to.path}") config.nix.registry; # This will additionally add your inputs to the system's legacy channels. Making legacy nix commands consistent as well, awesome!
    # package = pkgs.nixVersions.nix_2_23; # can't use, |>
    # package = pkgs.nixVersions.nix_2_24;
    package = pkgs.nixVersions.git;

    # Batch store deduplication, off the critical path.
    #
    # This is the GOOD knob. Do not re-enable `auto-optimise-store` in
    # flake.nix's nixConfig instead -- that one hardlinks on every store write
    # and slows every build; this timer does the same work in one batch.
    #
    # PREREQUISITE: the ext4 `large_dir` feature must be enabled on
    # /dev/nvme0n1p2 before this is useful. Without it the htree index on
    # /nix/store/.links caps out around ~12M entries (we hit it at ~9M) and the
    # daemon starts emitting
    #   cannot link "/nix/store/.links/...": No space left on device
    # on a filesystem with terabytes free. To enable, from rescue media with
    # the partition UNMOUNTED:
    #   tune2fs -O large_dir /dev/nvme0n1p2
    #   e2fsck -fD /dev/nvme0n1p2
    # Back up first; the flag is not cleanly reversible once a directory has
    # grown past two htree levels.
    #
    # Leaving this commented until that is done -- enabling it beforehand just
    # reproduces the ENOSPC flood. Flip to `true` after large_dir is live.
    # optimise.automatic = true;
    # optimise.dates = [ "weekly" ];
    # gc = {
    #   automatic = true;
    #   dates = "weekly";
    #   options = "--delete-older-than 400d";
    # };
  };
}
