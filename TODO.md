---
- mkdir -p ~/.config/oils       # for oshrc and yshrc
- mkdir -p ~/.local/share/oils  # for osh_history
- mainProgram = "osh"; needed? https://github.com/developing-today-forks/nixpkgs/blob/main/pkgs/by-name/oi/oils-for-unix/package.nix
- enable non-free firmware https://github.com/NixOS/nixpkgs/tree/master/nixos/modules/installer/scan
- https://discourse.nixos.org/t/use-lib-types-system-to-merge-attrsets-without-the-module-system/534/7
- https://theartofmachinery.com/2016/04/21/partitioned_live_usb.html
- add user ipaddr mac hostid machine-id ?containerid? other info? pull flake.outputs.hosts.<hostname> and put it in commit? to commits add list of files changed and number of lines changed total and in each file
- calculate time for rebuild and simple-rebuild
- generate a cuid during bootstrap and put it in /nix/persistent
- understand this https://serokell.io/blog/serokell-s-work-on-ghc-dependent-types-part-4
- touchid
- tmp
- waybar wpa_supplicant broke
- claude secret
- activation zed symlink
- zed config
- remove specialArgs and use modules somehow https://discourse.nixos.org/t/import-list-in-configuration-nix-vs-import-function/11372/5
- what is lib.composeManyExtensions and lib.makeExtensible
- standalone home manager is broken now, fix
-   nix = {
      # …
      registry = lib.mapAttrs (_: value: { flake = value; }) inputs;
      nixPath = lib.mapAttrsToList (key: value: "${key}=${value.to.path}") config.nix.registry;
    };
-   nix = {
  settings = {
    allowed-users = [ "@wheel" ];
    trusted-users = [ "root" "@wheel" ];
    experimental-features = [ "nix-command" "flakes" ];
  };
};

# Does not work without channels.
programs.command-not-found.enable = false;

users.mutableUsers = false;
- secrets
  - sops age key from ssh key for desktop use
  - github secret
    - github module
    - make 2 copies one for /root/auth one for /home/user/auth ?
    - use home manager?
  - sops age key from ssh key for desktop use
  - ssh key itself
- hosts defined by hostname but maybe they should be defined by hostid?
- users defined by username but maybe they should be defined by uid?
- persistence
  - rename /nix/persist ? /nix/persistent? /nix/var/persistent?
  - alias home dirs
  - alias /var/tmp
  - alias all of /var??
- turn on branch protection
- improve bootstrap
  - verbose trace possibly use nom, refer to rebuild.sh
  - home directory auto-permission each user
  - allow bootstrap vs regular
    - # TODO: make-bootstrap-versions
    - bootstrap hosts as <hostname>_bootstrap
    - rename hosts after bootstrap to <hostname>
    - auto-update from there
  - bootstrap with _bootstrap suffix hostname and configuration
    - nixos-rebuild switch "${hostname%_bootstrap}"
    - but why? is there a better alternative using peristent and post-install hooks to setup state?
    - or maybe this sets up the system and then waits for user or remote machine to trigger onlining?
    - avoiding a restart would be nice..
    - maybe using auto-upgrade or comin to choose a different target in bootstrap config?
  - lib.mkMerge vs lib.attrs.recursiveUpdate vs .extend()
  - rekey bootstrap
    - maybe
      - generate new key on desktop w/admin
      - add pk as admin keyed secret
      - sops add key to sops.yaml
      - rekey appropriate secrets
      - commit and push
      - bootstrap with specific group key instead of admin key
    - or maybe
      - use random /etc/ssh/ hostkey
      - generate age key
      - sops add key to sops.yaml
        - how to choose where to add?
      - replace /nix/persist/bootstrap/<key> with /etc/ssh key
      - commit and push rekeyed secrets
        - on conflict do what?
      - rm original bootstrap key
- ipxe

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

---
https://aldoborrero.com/posts/2023/01/15/setting-up-my-machines-nix-style/
boot.initrd.systemd.services.persisted-files = {

---
/
/home
/nix
/persist

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
# configure impermanence
environment.persistence."/persist" = {
  directories = [
    "/etc/nixos"
  ];
  files = [
    "/etc/machine-id"
    "/etc/ssh/ssh_host_ed25519_key"
    "/etc/ssh/ssh_host_ed25519_key.pub"
    "/etc/ssh/ssh_host_rsa_key"
    "/etc/ssh/ssh_host_rsa_key.pub"
};

security.sudo.extraConfig = ''
  # rollback results in sudo lectures after each reboot
  Defaults lecture = never
'';

---
isoImage.squashfsCompression = "gzip -Xcompression-level 1";

---
{
  # …

  # machine-id is used by systemd for the journal, if you don't
  # persist this file you won't be able to easily use journalctl to
  # look at journals for previous boots.
  environment.etc."machine-id".source
    = "/nix/persist/etc/machine-id";


  # if you want to run an openssh daemon, you may want to store the
  # host keys across reboots.
  #
  # For this to work you will need to create the directory yourself:
  # $ mkdir /nix/persist/etc/ssh
  environment.etc."ssh/ssh_host_rsa_key".source
    = "/nix/persist/etc/ssh/ssh_host_rsa_key";
  environment.etc."ssh/ssh_host_rsa_key.pub".source
    = "/nix/persist/etc/ssh/ssh_host_rsa_key.pub";
  environment.etc."ssh/ssh_host_ed25519_key".source
    = "/nix/persist/etc/ssh/ssh_host_ed25519_key";
  environment.etc."ssh/ssh_host_ed25519_key.pub".source
    = "/nix/persist/etc/ssh/ssh_host_ed25519_key.pub";

  # …
}

---
https://github.com/NixOS/nix/issues/1971
https://community.frame.work/t/nixos-on-the-framework-laptop-16/46743/48
https://xeiaso.net/blog/paranoid-nixos-2021-07-18/
https://mt-caret.github.io/blog/posts/2020-06-29-optin-state.html


---
{
  nix = {
    extraOptions = ''
      experimental-features = nix-command flakes
      !include ${config.sops.secrets.nixAccessTokens.path}
    '';
  };

  sops.secrets.nixAccessTokens = {
    mode = "0440";
    group = config.users.groups.keys.name;
  };
}
https://github.com/NixOS/nix/issues/6536#issuecomment-1254858889
https://lantian.pub/en/article/modify-computer/nixos-low-ram-vps.lantian/
boot.kernelParams = [
  # Disable auditing
  "audit=0"
  # Do not generate NIC names based on PCIe addresses (e.g. enp1s0, useless for VPS)
  # Generate names based on orders (e.g. eth0)
  "net.ifnames=0"
];
boot.initrd = {
  compressor = "zstd";
  compressorArgs = ["-19" "-T0"];
  systemd.enable = true;
};
boot.loader.grub = {
  enable = !config.boot.isContainer;
  default = "saved";
  devices = ["/dev/vda"];
};

# Manage networking with systemd-networkd
systemd.network.enable = true;
services.resolved.enable = false;
networking.nameservers = [
  "8.8.8.8"
];
      PermitRootLogin = lib.mkForce "prohibit-password";

      boot.initrd.postDeviceCommands = lib.mkIf (!config.boot.initrd.systemd.enable) ''
        # Set the system time from the hardware clock to work around a
        # bug in qemu-kvm > 1.5.2 (where the VM clock is initialised
        # to the *boot time* of the host).
        hwclock -s
      '';

      boot.initrd.availableKernelModules = [
        "virtio_net"
        "virtio_pci"
        "virtio_mmio"
        "virtio_blk"
        "virtio_scsi"
      ];
      boot.initrd.kernelModules = [
        "virtio_balloon"
        "virtio_console"
        "virtio_rng"
      ];

      disko = {
        # Do not let Disko manage fileSystems.* config for NixOS.
        # Reason is that Disko mounts partitions by GPT partition names, which are
        # easily overwritten with tools like fdisk. When you fail to deploy a new
        # config in this case, the old config that comes with the disk image will
        # not boot either.
        enableConfig = false;
        # Size for generated disk image. 2GB is enough for me. Adjust per your need.
         imageSize = "2G";
         # Path to disk. When Disko generates disk images, it actually runs a QEMU
         # virtual machine and runs the installation steps. Whether your VPS
         # recognizes its hard disk as "sda" or "vda" doesn't matter. We abide to
         # Disko's QEMU VM and use "vda" here.
         device = "/dev/vda";

         nodev."/" = {
           fsType = "tmpfs";
           mountOptions = ["relatime" "mode=755" "nosuid" "nodev"];
         };


         mountpoint = "/nix";
         mountOptions = ["compress-force=zstd" "nosuid" "nodev"];


         # Change to sda/vda based on how your VPS recognizes its hard drive
         cat result/main.raw | ssh root@123.45.678.90 "dd of=/dev/sda"

         If your rescue environment doesn't have SSH, use the following command: (ATTENTION: NO ENCRYPTION!)

         # Change to sda/vda based on how your VPS recognizes its hard drive
         # Run this on VPS
         nc -l 1234 | dd of=/dev/sda
         # Run this on local computer
         cat result/main.raw | nc 123.45.678.89 1234
https://nlewo.github.io/nixos-manual-sphinx/configuration/user-mgmt.xml.html
https://stackoverflow.com/questions/45144662/using-imports-with-argument-given-by-lib-mkoption
https://ryantm.github.io/nixpkgs/functions/library/lists/#function-library-lib.lists.unique


https://dataswamp.org/~solene/2022-08-03-nixos-with-live-usb-router.html
https://github.com/NixOS/nixos-hardware/blob/master/pcengines/apu/default.nix
https://euank.com/2023/02/22/v6-plus.html

http://web.archive.org/web/20220125141040/https://lastlog.de/blog/posts/apu.html
https://francis.begyn.be/blog/nixos-home-router
https://pavluk.org/blog/2022/01/26/nixos_router.html
https://github.com/cleverca22/not-os
https://gti.telent.net/dan/liminix
https://github.com/disassembler/network/blob/master/nixos/portal/configuration.nix
https://gti.telent.net/dan/liminix
https://www.jjpdev.com/posts/home-router-nixos/
https://www.jjpdev.com/posts/home-network/
https://www.jjpdev.com/posts/home-network-live/
https://www.jjpdev.com/posts/home-network-wireguard/
https://github.com/MakiseKurisu/dewclaw?tab=readme-ov-file
https://skogsbrus.xyz/building-a-router-with-nixos/
https://github.com/breakds/nixos-routers/blob/main/machines/welderhelper/router.nix
https://github.com/stanipintjuk/nixos-router/blob/master/mkRouter.nix
https://gitlab.com/simple-nixos-mailserver/nixos-mailserver/
https://github.com/chayleaf/nixos-router?tab=readme-ov-file
