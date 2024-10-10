
---
- 1tb disko
- wpa_supplicant waybar
- persistence
  - btrfs|zfs|tmpfs
  - sops.age.keyFile # replace default with persistence
- improve bootstrap
- ipxe
- imperative wpa_supplicant
  - wpa_supplicant_gui

---
https://0xda.de/blog/2024/06/framework-and-nixos-secure-boot-day-three/
https://0xda.de/blog/2024/07/framework-and-nixos-sops-nix-secrets-management/
https://github.com/clan-lol/clan-core/blob/a95853276605332edd7bf109d9dce87a3c66a02e/nixosModules/clanCore/facts/secret/sops.nix#L44-L46
https://github.com/garnix-io/template-rss-bridge
https://github.com/garnix-io/template-ssh-app
https://github.com/garnix-io/template-ttrss?tab=readme-ov-file
https://github.com/golang/go/issues/21498
https://github.com/jhvst/nix-config/blob/d3f7b0836c3f7ba34e3067964608fa8884fbc255/nixosConfigurations/starlabs/default.nix#L260
https://github.com/majbacka-labs/nixos.fi
https://github.com/Mic92/sops-nix/issues/378#issuecomment-2068820729
https://github.com/Mic92/sops-nix/issues/622#issuecomment-2351778124
https://github.com/Mic92/sops-nix/pull/417
https://github.com/NixOS/nixpkgs/issues/111252
https://github.com/NixOS/nixpkgs/pull/113716
https://github.com/NixOS/nixpkgs/pull/75800
https://github.com/thiagokokada/nix-configs/
https://github.com/viperML/nh
https://github.com/vst/opsops
https://grahamc.com/blog/erase-your-darlings/
https://joeduffyblog.com/2016/02/07/the-error-model/
https://willbush.dev/blog/impermanent-nixos/
https://www.tweag.io/blog/2023-02-09-nixos-vm-on-macos/

---
networking.nftables.enable = true;

---
system.switch = {
  enable = false;
  enableNg = true;
};

---
boot.initrd.systemd

---
services.pipewire = {
  enable = true;
  alsa.enable = true;
  pulse.enable = true;
  # jack.enable = true;
};

---
security.rtkit.enable = true;

---
# not networkmanager, but compare iwd and wpa_supplicant
networking.networkmanager.wifi.backend = "iwd"

---
boot.tmp.useTmpfs = true;
systemd.services.nix-daemon = {
  environment.TMPDIR = "/var/tmp";
};

---
zramSwap = {
  enable = true;
  algorithm = "zstd"; # lz4 or zstd
};

---
services.fstrim.enable = true;
boot.binfmt.emulatedSystems = [ "aarch64-linux" "riscv64-linux" ];
services.dbus.implementation = "broker"
services.irqbalance # only for slow things, not 10gbe

---
nix.gc = {
  automatic = true;
  randomizedDelaySec = "14m";
  options = "--delete-older-than 30d";
};

---
system.autoUpgrade = {
  # https://nixos.org/manual/nixos/stable/index.html#sec-upgrading-automatic
  enable = true;
  allowReboot = true;
}
https://github.com/Misterio77/nix-config/blob/74311ba/modules/nixos/hydra-auto-upgrade.nix#L79

---
nvme0n1
gpt
32gb free
32gb fat /boot
326,143,836,160 bytes free
1tb /nix
1tb /persist

---
{
  fileSystems."/" = {
    device = "none";
    fsType = "tmpfs";
    options = [ "defaults" "size=25%" "mode=755" ];
  };

  fileSystems."/persistent" = {
    device = "/dev/root_vg/root";
    neededForBoot = true;
    fsType = "btrfs";
    options = [ "subvol=persistent" ];
  };

  fileSystems."/nix" = {
    device = "/dev/root_vg/root";
    fsType = "btrfs";
    options = [ "subvol=nix" ];
  };

  fileSystems."/boot" = {
    device = "/dev/disk/by-uuid/XXXX-XXXX";
    fsType = "vfat";
  };
}

---
fileSystems."/" =
    { device = "none";
      fsType = "tmpfs";
      options = [ "size=3G" "mode=755" ]; # mode=755 so only root can write to those files
    };
  fileSystems."/home/username" =
    { device = "none";
      fsType = "tmpfs";  # Can be stored on normal drive or on tmpfs as well
      options = [ "size=4G" "mode=777" ];
    };
  fileSystems."/nix" =  # can be LUKS encrypted
    { device = "/dev/disk/by-uuid/UUID";
      fsType = "ext4";
    };
  fileSystems."/boot" =
    { device = "/dev/disk/by-uuid/UUID";
      fsType = "vfat";
    };

---
{ config, pkgs, ... }:
let
  impermanence = builtins.fetchTarball "https://github.com/nix-community/impermanence/archive/master.tar.gz";
in
{
  imports = [ "${impermanence}/nixos.nix" ];

  environment.persistence."/nix/persist/system" = {
    hideMounts = true;
    directories = [
      "/var/log"
      "/var/lib/bluetooth"
      "/var/lib/nixos"
      "/var/lib/systemd/coredump"
      "/etc/NetworkManager/system-connections"
      { directory = "/var/lib/colord"; user = "colord"; group = "colord"; mode = "u=rwx,g=rx,o="; }
    ];
    files = [
      "/etc/machine-id"
      { file = "/etc/nix/id_rsa"; parentDirectory = { mode = "u=rwx,g=,o="; }; }
    ];
  };
}

---
/boot, /nix, /var/log, /home - self-explanatory
/tmp - for large builds (so they don't get put on tmpfs), gets cleaned on reboot if you set boot.tmp.cleanOnBoot
/var/tmp - just a good idea to not have this on tmpfs
/var/lib/systemd - systemd stuff, not sure if necessary but definitely won't hurt, it's quite small anyway
/etc/nixos - system config
/var/lib/nixos - important nixos files like uid/gid map
/etc/adjtime - something about hardware clock offset
/etc/machine-id - needed for systemd logs and possibly other stuff
...as well as the dirs for all the services. You probably want to add /var/db/dhcpcd and /var/db/sudo/lectured.

---
{
  fileSystems."/" = {
    device = "/dev/root_vg/root";
    fsType = "btrfs";
    options = [ "subvol=root" ];
  };

  boot.initrd.postDeviceCommands = lib.mkAfter ''
    mkdir /btrfs_tmp
    mount /dev/root_vg/root /btrfs_tmp
    if [[ -e /btrfs_tmp/root ]]; then
        mkdir -p /btrfs_tmp/old_roots
        timestamp=$(date --date="@$(stat -c %Y /btrfs_tmp/root)" "+%Y-%m-%-d_%H:%M:%S")
        mv /btrfs_tmp/root "/btrfs_tmp/old_roots/$timestamp"
    fi

    delete_subvolume_recursively() {
        IFS=$'\n'
        for i in $(btrfs subvolume list -o "$1" | cut -f 9- -d ' '); do
            delete_subvolume_recursively "/btrfs_tmp/$i"
        done
        btrfs subvolume delete "$1"
    }

    for i in $(find /btrfs_tmp/old_roots/ -maxdepth 1 -mtime +30); do
        delete_subvolume_recursively "$i"
    done

    btrfs subvolume create /btrfs_tmp/root
    umount /btrfs_tmp
  '';

  fileSystems."/persistent" = {
    device = "/dev/root_vg/root";
    neededForBoot = true;
    fsType = "btrfs";
    options = [ "subvol=persistent" ];
  };

  fileSystems."/nix" = {
    device = "/dev/root_vg/root";
    fsType = "btrfs";
    options = [ "subvol=nix" ];
  };

  fileSystems."/boot" = {
    device = "/dev/disk/by-uuid/XXXX-XXXX";
    fsType = "vfat";
  };
}
