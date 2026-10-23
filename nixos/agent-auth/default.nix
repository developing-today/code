{ lib, pkgs, ... }:
# Hidden-shell auth: every shell for `user` -- including the non-interactive
# `bash -c` shells agents, systemd user units, t3code, opencode and ssh spawn --
# starts already logged in to Bitwarden Secrets Manager (bws), the 1Password
# CLI and Jules, with the Cloudflare AI-gateway vars set.
#
# How:
#   * BASH_ENV -> agent-shell-env hook. A non-interactive, non-login bash reads
#     no startup file except $BASH_ENV, so this is the only reliable hook.
#     environment.sessionVariables goes through pam_env, which reaches every
#     PAM session: SDDM/Hyprland, ssh, and `systemd --user` itself. home/common
#     also sets it in systemd.user.sessionVariables and ~/.bashrc as backups.
#   * Secrets: sops-nix decrypts them at activation into
#     /run/secrets/agent-auth/<name> (owner `user`, 0400). The hook reads them
#     at shell start; only the PATHS are in the Nix store.
#       bws-access-token        -> BWS_ACCESS_TOKEN (bws login; no state needed)
#       jules-api-key           -> JULES_API_KEY
#       cloudflare-*            -> CLOUDFLARE_{ACCOUNT_ID,GATEWAY_ID,API_TOKEN}
#       op-service-account-token -> OP_SERVICE_ACCOUNT_TOKEN (not declared yet:
#                                   no token in sops; hook falls back to
#                                   ~/.config/op/service-account-token)
#   * Every bws secret is also mirrored into secrets/sops/common/bws.yaml by
#     lib/bws-to-sops.sh; point more sops.secrets at it with `key = "<bws key>"`.
#   * 1Password: service accounts cannot see Private/Personal vaults;
#     `op-personal` drops the token and goes through the desktop app
#     integration enabled below.
#   * Jules: the CLI only supports browser OAuth (`jules login`), stored in
#     gnome-keyring via go-keyring. gnome-keyring is unlocked by PAM at login
#     (GNOME module), and the hook guarantees DBUS_SESSION_BUS_ADDRESS so the
#     token is reachable from any shell. One-time: `agent-auth login`.
#
# Check from any shell: `agent-auth status`.
let
  agent-shell-env = pkgs.callPackage (lib.from-root "pkgs/agent-shell-env") { };
  userSecret = sopsFile: key: {
    inherit key;
    sopsFile = lib.from-root sopsFile;
    owner = "user";
    mode = "0400";
  };
in
{
  sops.secrets = {
    "agent-auth/bws-access-token" = userSecret "secrets/sops/common/bitwarden.yaml" "bws_access_token";
    "agent-auth/jules-api-key" = userSecret "secrets/sops/common/bws.yaml" "google.jules-api-key";
    "agent-auth/cloudflare-account-id" =
      userSecret "secrets/sops/common/cloudflare.yaml" "cloudflare_account_id";
    "agent-auth/cloudflare-gateway-id" =
      userSecret "secrets/sops/common/cloudflare.yaml" "cloudflare_gateway_id";
    "agent-auth/cloudflare-ai-inference-token" =
      userSecret "secrets/sops/common/cloudflare.yaml" "cloudflare_api_token_ai_inference";
    "agent-auth/muse-sdk-token" = userSecret "secrets/sops/common/bws.yaml" "muse.sdk-token";
  };

  # `op` as a setgid (onepassword-cli) wrapper in /run/wrappers/bin -- required
  # for the desktop-app integration; harmless for service-account use.
  programs._1password.enable = true;
  # Replaces the bare `_1password-gui` systemPackages entry. polkitPolicyOwners
  # lets the app authorise CLI integration and system-auth unlock for `user`.
  programs._1password-gui = {
    enable = true;
    polkitPolicyOwners = [ "user" ];
  };

  environment.systemPackages = [ agent-shell-env ];
  environment.sessionVariables.BASH_ENV = "${agent-shell-env}/share/agent-shell-env/env.sh";
}
