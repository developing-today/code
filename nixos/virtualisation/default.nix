{ pkgs, ... }:

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
    #   { from = "host"; host.port = 8080; guest.port = 80; }  # HTTP example
    # ];

    # Option 2: Virtual network interface (recommended for ping)
    # vlans = [ 1 ];
    # networks = {
    #   "1" = {
    #     # name = "vnet"; # Optional: custom interface name
    #     # addressPrefix = "192.168.100";  # Creates 192.168.100.0/24 network
    #     # localAddress = "192.168.100.10";  # Optional: specific guest IP
    #     # hostAddress = "192.168.100.1";  # Optional: specific host IP
    #     # vlan = 1;
    #   };
    # };
  };

  # libvirtd >= 12 seals its secrets-encryption key with systemd
  # credentials: the package's own 10-secret.conf drop-in loads
  # /var/lib/libvirt/secrets/secrets-encryption-key as a
  # LoadCredentialEncrypted, which systemd can only decrypt with the
  # host credential key (/var/lib/systemd/credential.secret).
  #
  # Nothing on NixOS creates that host key eagerly, and if it is absent
  # libvirtd fails at the CREDENTIALS step ("Failed to determine local
  # credential key") -- which also makes `nixos-rebuild switch` exit 4
  # even when the rest of the activation succeeded. Worse, the libvirt
  # key blob is encrypted under whatever host key existed when
  # virt-secret-init-encryption first ran; if that host key is lost,
  # the blob is undecryptable forever.
  #
  # Observed live on amd 2026-10-26: the blob dated 2026-10-20 could
  # not be decrypted because the host key had vanished. Recovery (safe
  # only while no libvirt secrets are defined -- check
  # /var/lib/libvirt/secrets for *.xml first, and expect to lose any
  # secret encrypted under the old master key):
  #   sudo systemd-creds setup
  #   sudo mv /var/lib/libvirt/secrets/secrets-encryption-key{,.undecryptable}
  #   sudo systemctl start virt-secret-init-encryption.service
  #   sudo systemctl start libvirtd.service
  #
  # This oneshot keeps the host key in existence from the first
  # activation onward, so the libvirt blob is always created under a
  # persistent key. `systemd-creds setup` is idempotent; the condition
  # makes it a no-op once the key exists.
  systemd.services.creds-host-key-setup = {
    description = "Ensure the systemd credentials host key exists (libvirtd secrets decryption depends on it)";
    wantedBy = [ "multi-user.target" ];
    before = [
      "virt-secret-init-encryption.service"
      "libvirtd.service"
    ];
    unitConfig.ConditionPathExists = "!/var/lib/systemd/credential.secret";
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      ExecStart = "${pkgs.systemd}/bin/systemd-creds setup";
    };
  };
}
