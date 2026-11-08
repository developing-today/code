{ pkgs, ... }:

let
  # Creates /var/lib/libvirt/secrets/secrets-encryption-key.plain, once.
  # First choice is migrating the key out of the existing sealed blob
  # (possible only while the blob still decrypts); otherwise generate
  # the same material virt-secret-init-encryption produces: 32 random
  # bytes. See the long comment above the units for why the key lives
  # in plaintext at all.
  virtSecretPlainKey = pkgs.writeShellScript "virt-secret-plain-key" ''
    set -eu
    plain=/var/lib/libvirt/secrets/secrets-encryption-key.plain
    blob=/var/lib/libvirt/secrets/secrets-encryption-key
    if [ -e "$blob" ] && ${pkgs.systemd}/bin/systemd-creds decrypt --name=secrets-encryption-key "$blob" "$plain" 2>/dev/null; then
      echo "virt-secret-plain-key: migrated the existing libvirt secrets master key out of its sealed blob"
    else
      rm -f "$plain"
      ${pkgs.coreutils}/bin/dd if=/dev/random status=none bs=32 count=1 of="$plain"
      echo "virt-secret-plain-key: generated a fresh libvirt secrets master key; libvirt secrets sealed under any previous key are unrecoverable"
    fi
    chmod 600 "$plain"
  '';
in
{
  virtualisation = {
    libvirtd.enable = true;
    libvirtd.qemu.runAsRoot = true;
    libvirtd.allowedBridges = [
      "virbr0"
      "br0"
    ];
    docker.enable = true;

    # Option 1: Port forwarding
    # forwardPorts = [
    #   { from = "host"; host.port = 2222; guest.port = 22; }  # SSH
    #   { from = "host"; host.port = 8080; guest.port = 8080 }  # HTTP example
    # ];

    # Option 2: Virtual network interface (recommended for ping)
    # vlans = [ 1 ];
    # networks = {
    #   "1" = {
    #     # name = "vnet"; # Optional: custom interface name
    #     # addressPrefix = "192.168.100";  # Creates "192.168.100.x" network
    #     # localAddress = "192.168.100.10";  # Specific IP in the prefix
    #     # hostAddress = "192.168.100.1";  # Specific IP in the prefix
    #     # vlan = 1;
    #   };
    # };
  };

  # libvirtd's secrets master key, delivered WITHOUT systemd credential
  # encryption -- deliberately (code#5449).
  #
  # libvirt >= 12 wants its master key as a systemd credential named
  # "secrets-encryption-key" (the daemons find it via
  # $SECRETS_ENCRYPTION_KEY, pointing into $CREDENTIALS_DIRECTORY).
  # Upstream's delivery seals the key: the libvirt package's
  # 10-secret.conf drop-in (and virtsecretd's unit) load
  # /var/lib/libvirt/secrets/secrets-encryption-key via
  # LoadCredentialEncrypted, and systemd decrypts it with the host
  # credential key, /var/lib/systemd/credential.secret.
  #
  # That host key is bound to the machine ID, and on this host the
  # machine ID is randomized at every boot: /etc/machine-id is not
  # persisted (impermanence persists /home, /root, /var, /tmp and the
  # SSH host key only). systemd deletes credential.secret as foreign
  # at every boot and mints a new one, so every blob sealed under a
  # previous boot's key is undecryptable -- libvirtd died at the
  # CREDENTIALS step (status 243) at the 2026-10-26 and 2026-11-04
  # boots, and the generation switch after the latter recorded exit 4
  # for it. The recovery ritual that used to live here (re-seal the
  # blob under each boot's new host key) treated the symptom per boot.
  #
  # Persisting the machine ID would stabilize the sealing, but it
  # answers the wrong question (Drew, 2026-11-04): why should libvirt
  # care about the machine ID at all? The blob and the host key both
  # live in persisted /var, side by side -- the encryption only ever
  # protected the key from a reader who could already read the key
  # that decrypts it. So the sealing is removed instead:
  #
  #   * The serviceConfig overrides below land in each daemon's
  #     module-generated overrides.conf drop-in, which systemd applies
  #     after the package's 10-secret.conf. The empty
  #     LoadCredentialEncrypted assignment resets the sealed delivery
  #     (systemd drop-in semantics); LoadCredential then hands the
  #     daemon the same 32-byte key, in the clear, from a root-only
  #     file in persisted /var. Credential name and
  #     $SECRETS_ENCRYPTION_KEY are unchanged, and no host credential
  #     key is involved at any point, so machine-ID churn is
  #     irrelevant to libvirtd from now on.
  #   * virt-secret-plain-key.service (below) creates that file once:
  #     it migrates the existing sealed blob when it can still be
  #     decrypted, and otherwise generates 32 fresh random bytes.
  #     Fresh bytes mean libvirt secrets sealed under a previous
  #     master key are lost -- check /var/lib/libvirt/secrets for
  #     *.xml before treating that as free. (None are defined on amd
  #     today; if the plain file is ever lost, deleting the situation
  #     back to "file missing" and restarting the generator + the
  #     daemons is the whole recovery.)
  #
  # The creds-host-key-setup oneshot that used to live here existed
  # only to keep a host credential key in existence for this one
  # decryption; it is removed with its only consumer. The package's
  # virt-secret-init-encryption.service still runs (the package drop-in
  # Requires it), but it only maintains the now-unused sealed blob.
  systemd.services.libvirtd.serviceConfig = {
    # Empty value renders as a bare "LoadCredentialEncrypted=" line:
    # the drop-in reset that drops the package's sealed delivery.
    LoadCredentialEncrypted = "";
    LoadCredential = [
      "secrets-encryption-key:/var/lib/libvirt/secrets/secrets-encryption-key.plain"
    ];
  };

  systemd.services.virtsecretd.serviceConfig = {
    LoadCredentialEncrypted = "";
    LoadCredential = [
      "secrets-encryption-key:/var/lib/libvirt/secrets/secrets-encryption-key.plain"
    ];
  };

  systemd.services.virt-secret-plain-key = {
    description = "Create libvirtd's plaintext secrets master key (once, if missing)";
    wantedBy = [ "multi-user.target" ];
    before = [
      "libvirtd.service"
      "virtsecretd.service"
    ];
    unitConfig.ConditionPathExists = "!/var/lib/libvirt/secrets/secrets-encryption-key.plain";
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      UMask = "0077";
      ExecStart = "${virtSecretPlainKey}";
    };
  };
}
