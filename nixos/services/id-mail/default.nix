{
  config,
  lib,
  pkgs,
  inputs,
  ...
}:
let
  cfg = config.services.id-mail;

  mailSender = pkgs.writeShellApplication {
    name = "id-cloudflare-mail";
    runtimeInputs = [
      pkgs.curl
      pkgs.jq
    ];
    text = builtins.readFile ./cloudflare-mail.sh;
  };

  mailEnv =
    if cfg.transport == "rest" then
      ''
        ID_MAIL_TRANSPORT=rest
        CLOUDFLARE_API_TOKEN=${config.sops.placeholder.cloudflare_email_sending_token}
        CLOUDFLARE_ACCOUNT_ID=${config.sops.placeholder.cloudflare_account_id}
      ''
    else
      ''
        ID_MAIL_TRANSPORT=worker
        ID_MAIL_WORKER_URL=${cfg.workerUrl}
        ID_MAIL_WORKER_TOKEN=${config.sops.placeholder.id_mail_worker_token}
      '';
in
{
  imports = [ (lib.from-root "pkgs/id/nix/id-module.nix") ];

  options.services.id-mail = {
    transport = lib.mkOption {
      type = lib.types.enum [
        "rest"
        "worker"
      ];
      default = "rest";
      description = "How confirmation mail leaves: the Cloudflare Email Sending REST API, or the id-mail Worker's send_email binding.";
    };

    workerUrl = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "URL of the id-mail Worker. Required when transport is worker.";
    };
  };

  config = {
    assertions = [
      {
        assertion = cfg.transport != "worker" || cfg.workerUrl != null;
        message = "services.id-mail.workerUrl must be set for the worker transport";
      }
    ];

    sops.secrets = {
      cloudflare_email_sending_token.sopsFile = lib.from-root "secrets/sops/common/cloudflare.yaml";
      cloudflare_account_id.sopsFile = lib.from-root "secrets/sops/common/cloudflare.yaml";
      id_mail_worker_token.sopsFile = lib.from-root "secrets/sops/common/cloudflare.yaml";
    };

    sops.templates."id-mail.env".content = mailEnv;

    services.id = {
      package = inputs.self.packages.${pkgs.stdenv.hostPlatform.system}.id-web;
      instances.primary = {
        enable = true;
        extraArgs = [
          "--world-mail-command"
          "${mailSender}/bin/id-cloudflare-mail"
        ];
      };
    };

    systemd.services.id-primary.serviceConfig.EnvironmentFile =
      config.sops.templates."id-mail.env".path;
  };
}
