https://github.com/Misterio77/nix-config/blob/9957575d5a55bf38fd2a47e0729c89b64062b293/hosts/common/optional/wireless.nix
https://www.reddit.com/r/NixOS/comments/1fnkbj5/sops_and_wireless_credentials/
sops.secrets.wireless = {
  sopsFile = ../secrets.yaml;
  neededForUsers = true;
};

networking.wireless = {
  enable = true;
  fallbackToWPA2 = false;
  # Declarative
  secretsFile = config.sops.secrets.wireless.path;
  networks = {
    "JVGCLARO" = {
      pskRaw = "ext:jvgclaro";
    };
error:
Failed assertions:
- The option definition `networking.wireless.environmentFile' in `/nix/store/dzn3lfkkbiz6rr03i04g1al4m10zbh7c-source/hosts/configuration.nix' no longer has any effect; please remove it.
Secrets are now handled by the `networking.wireless.secretsFile` and
`networking.wireless.networks.<name>.pskRaw` options.
The change is motivated by a mechanism recently added by wpa_supplicant
itself to separate secrets from configuration, making the previous
method obsolete.

The syntax of the `secretsFile` is the same as before, except the
values are interpreted literally, unlike environment variables.
To update, remove quotes or character escapes, if necessary, and
apply the following changes to your configuration:
  {
    home.psk = "@psk_home@";          →  home.pskRaw = "ext:psk_home";
    other.pskRaw = "@psk_other@";     →  other.pskRaw = "ext:psk_other";
    work.auth = ''
      eap=PEAP
      identity="my-user@example.com"
      password=@pass_work@            →  password=ext:pass_work
    '';
  }


- You can not use networking.networkmanager with networking.wireless.
Except if you mark some interfaces as <literal>unmanaged</literal> by NetworkManager.
┏━ 1 Errors:
⋮
┃        - You can not use networking.networkmanager with networking.wireless.
┃        Except if you mark some interfaces as <literal>unmanaged</literal> by NetworkMana…
┣━━━
┗━ ∑ ⚠ Exited with 1 errors reported by nix at 00:32:56 after 4s
osh-0.22.0$

https://kokada.dev/blog/an-unordered-list-of-things-i-miss-in-go/
https://kokada.dev/blog/an-unordered-list-of-hidden-gems-inside-nixos/
https://kokada.dev/blog/go-a-reasonable-good-language/
https://github.com/danthegoodman1/BreakingSQLite

# Enables DHCP on each ethernet and wireless interface. In case of scripted networking
# (the default) this is the recommended approach. When using systemd-networkd it's
# still possible to use this option, but it's recommended to use it in conjunction
# with explicit per-interface declarations with `networking.interfaces.<interface>.useDHCP`.
networking.useDHCP = lib.mkDefault true;
# networking.interfaces.enp1s0.useDHCP = lib.mkDefault true;

https://dee.underscore.world/blog/installing-nixos-unconventionally/

https://garnix.io/blog/hosting-nixos
https://github.com/garnix-io/template-jitsi
https://github.com/garnix-io?q=template&type=all&language=&sort=
https://garnix.io/docs/hosting/persistence

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

nmcli device wifi connect MY_WIFI_SSID password thepasswordisonthefridge
age
  users = {
    mutableUsers = false;
    };
    https://github.com/developing-today-forks/nixos-unattended-installer/tree/main
    my/secret/key
    my: secret: key: '123'

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
    https://github.com/nix-community/disko/blob/574400001b3ffe555c7a21e0ff846230759be2ed/docs/disko-install.md?plain=1#L120

    cat /home/user/.config/sops/age/keys.txt
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
