{
  lib,
  ...
}:
{
  imports = [
    (lib.from-root "nixos/users")
  ];

  # Dot SSH (Aster's cloud VM identity): a dedicated, non-sudo,
  # key-only account whose only login path is the dot-ssh Cloudflare
  # Tunnel (dot-ssh.security.cab -> ssh://localhost:22) gated by a
  # dedicated Cloudflare Access service token. Drew approved the build
  # 2026-11-16 (draft in #alignment, thread 1794871876.785569).
  #
  # No password (hashedPassword "*" — same convention as root here),
  # no groups (no wheel, so no sudo), no authorized keys yet: Aster's
  # VM public key gets declared in openssh.authorizedKeys.keys when it
  # arrives, and the command restriction lands on that key line as a
  # forced command (plus the Match block below as server-side belt and
  # braces, so the restrictions hold regardless of key options).
  users.users.dot-ssh = {
    uid = 1339;
    isNormalUser = true;
    hashedPassword = "*";
    description = "Dot (Aster) restricted SSH";
    extraGroups = [ ];
    openssh.authorizedKeys.keys = [ ];
  };

  services.openssh.extraConfig = ''
    Match User dot-ssh
      PasswordAuthentication no
      KbdInteractiveAuthentication no
      AllowTcpForwarding no
      AllowAgentForwarding no
      X11Forwarding no
      PermitTunnel no
  '';
}
