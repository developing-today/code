

Use networking.wireless.environmentFile:

  sops.secrets."wireless.env" = { };
  networking.wireless.environmentFile = config.sops.secrets."wireless.env".path;
  networking.wireless.networks = {
    "@home_uuid@" = {
      psk = "@home_psk@";
    };
  };

And in your .sops.yaml:

wireless.env: |
   home_uuid=foo
   home_psk=secret

I'm doing that in my dotfiles

---

networking = {
 hostName = hostname;
 wireless.enable = true;
 wireless.scanOnLowSignal = false;
 wireless.networks = {
 "${config.sops.secrets."networking/home/ssid".val}" = {
 hidden = true;
 psk = config.sops.secrets."networking/home/psk".val;
 authProtocols = \["WPA-PSK"\];
 };
};

---


nix build .#nixosConfigurations.unattended-installer_amd.config.system.build.isoImage

unattended install
wormhole or portal
persistence
btrfs or zfs
sops.age.keyFile
nmcli device wifi connect MY_WIFI_SSID password thepasswordisonthefridge
https://0xda.de/blog/2024/07/framework-and-nixos-sops-nix-secrets-management/
age
  users = {
    mutableUsers = false;
    };
nvme0n1
gpt
32gb free
32gb fat /boot
326,143,836,160 bytes free
1tb /nix
1tb /persist
https://github.com/developing-today-forks/nixos-unattended-installer/tree/main
https://grahamc.com/blog/erase-your-darlings/
https://0xda.de/blog/2024/07/framework-and-nixos-sops-nix-secrets-management/
https://0xda.de/blog/2024/06/framework-and-nixos-secure-boot-day-three/
https://github.com/vst/opsops
my/secret/key
my: secret: key: '123'
https://github.com/Mic92/sops-nix/issues/378#issuecomment-2068820729
https://github.com/clan-lol/clan-core/blob/a95853276605332edd7bf109d9dce87a3c66a02e/nixosModules/clanCore/facts/secret/sops.nix#L44-L46
https://github.com/Mic92/sops-nix/pull/417
https://github.com/Mic92/sops-nix/issues/622#issuecomment-2351778124
{
  disko.devices = {
    disk = {
      main = {
        device = "/dev/disk/by-id/some-disk-id";
        type = "disk";
        content = {
          type = "gpt";
          partitions = {
            ESP = {
              type = "EF00";
              size = "500M";
              content = {
                type = "filesystem";
                format = "vfat";
                mountpoint = "/boot";
                mountOptions = [ "umask=0077" ];
              };
            };
            root = {
              size = "100%";
              content = {
                type = "filesystem";
                format = "ext4";
                mountpoint = "/";
              };
            };
          };
        };
      };
    };
  };
}
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



    /boot, /nix, /var/log, /home - self-explanatory

    /tmp - for large builds (so they don't get put on tmpfs), gets cleaned on reboot if you set boot.tmp.cleanOnBoot

    /var/tmp - just a good idea to not have this on tmpfs

    /var/lib/systemd - systemd stuff, not sure if necessary but definitely won't hurt, it's quite small anyway

    /etc/nixos - system config

    /var/lib/nixos - important nixos files like uid/gid map

    /etc/adjtime - something about hardware clock offset

    /etc/machine-id - needed for systemd logs and possibly other stuff

    ...as well as the dirs for all the services. You probably want to add /var/db/dhcpcd and /var/db/sudo/lectured.

https://github.com/nix-community/disko/blob/574400001b3ffe555c7a21e0ff846230759be2ed/docs/disko-install.md?plain=1#L120

https://www.tweag.io/blog/2023-02-09-nixos-vm-on-macos/

cat /home/user/.config/sops/age/keys.txt

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

https://willbush.dev/blog/impermanent-nixos/
https://github.com/magic-wormhole/magic-wormhole
https://forums.whonix.org/t/magic-wormhole-easyly-get-things-from-one-computer-to-another-safely-review/4026
https://magic-wormhole.readthedocs.io/en/latest/
https://sendfiles.dev/
portal
croc
https://github.com/samyk/slipstream
https://github.com/samyk/pwnat
https://github.com/schollz/croc
https://file.pizza/
https://nitter.privacydev.net/awesomekling/status/1822241531501162806#m
https://winden.app/s
https://wormhole.app/
https://github.com/magic-wormhole/magic-wormhole/blob/master/docs/attacks.rst
magic-wormhole
magic-wormhole-rs
relay
mailbox
https://tailscale.com/blog/how-nat-traversal-works
https://github.com/SpatiumPortae/portal
https://github.com/developing-today-forks/nixos-unattended-installer/tree/main
https://grahamc.com/blog/erase-your-darlings/
https://github.com/psanford/wormhole-william
https://github.com/Jacalz/rymdport
https://tailscale.com/blog/how-nat-traversal-works
magic-wormhole
: wormhole-william
: magic-wormhole-rs
portal
mari0
