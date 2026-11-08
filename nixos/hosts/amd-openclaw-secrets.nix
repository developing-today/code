{ lib, ... }:
let
  userSecret = key: {
    inherit key;
    sopsFile = lib.from-root "secrets/sops/common/bws.yaml";
    owner = "user";
    mode = "0400";
  };
in
{
  # OpenClaw gateway + channel tokens. Bitwarden Secrets Manager is the
  # source of truth; lib/bws-to-sops.sh mirrors every bws secret into
  # secrets/sops/common/bws.yaml (age-encrypted to this host). sops-nix
  # decrypts these at system activation to /run/secrets/openclaw/*,
  # where the openclaw-gateway user service reads them through the
  # wrapper's file-based environment (see home/openclaw).
  sops.secrets = {
    "openclaw/gateway-token" = userSecret "openclaw.gateway-token";
    "openclaw/slack-bot-token" = userSecret "openclaw.slack-bot-token";
    "openclaw/slack-app-token" = userSecret "openclaw.slack-app-token";
  };
}
