# PM apps (code#1752): the remaining PM-stack servers — Kaneo, its
# PostgreSQL, Grist, Pad, and ready-for-agent — declared in this flake
# like everything else on this host, replacing the hand-run deployment
# in ~/pm-stack (three docker compose projects plus one hand-written
# user unit) that was not boot-enabled and would not have come back
# after a reboot.
#
# Fidelity rules — this module reproduces the compose setup exactly:
# - Same images, pinned the same way (Kaneo by tag; Grist and Pad by
#   tag + digest, the digest being the binding pin).
# - Same ports: Kaneo loopback only (public access arrives through
#   the Cloudflare Tunnel, never directly); Grist and Pad on loopback
#   plus the host's tailnet address; ready-for-agent on 6056.
# - Same data, carried IN PLACE: the containers mount the same docker
#   named volumes the compose projects created
#   (pm-stack_postgres_data, grist_grist_data, pad_pad_data), so no
#   data moves and nothing is re-seeded.
# - Same configuration: each container reads the same on-machine
#   .env file its compose project used (~/pm-stack/.env,
#   ~/pm-stack/grist/.env, ~/pm-stack/pad/.env — mode 600, rendered
#   by ~/pm-stack's deploy.sh from the pm-stack SOPS file; never
#   hand-edited and never in this repo).
#
# Activation (part of the NixOS switch that lands this module):
#   1. Stop the compose projects, WITHOUT removing volumes:
#        docker compose -f ~/pm-stack/compose.yml down
#        docker compose -f ~/pm-stack/grist/compose.yml down
#        docker compose -f ~/pm-stack/pad/compose.yml down
#   2. nixos-rebuild switch. home-manager renames the hand-written
#      ready-for-agent unit to *.backup, as it did for pm-stack.
#   3. Verify 5173, 8484, 7777, and 6056 all answer with the
#      pre-switch data intact.
#
# Differences from compose, deliberately accepted: compose
# healthchecks have no oci-containers equivalent, and kaneo's
# depends_on was health-gated while dependsOn here is start-order
# only — Kaneo's own reconnect behavior covers the PostgreSQL
# warm-up window.
{
  config,
  lib,
  ...
}:
let
  cfg = config.services.pm-apps;
  home = "/home/${cfg.user}";
  pmDir = "${home}/pm-stack";
in
{
  options.services.pm-apps = {
    enable = lib.mkEnableOption "the PM app servers (Kaneo + PostgreSQL, Grist, Pad, ready-for-agent) as NixOS-managed services";

    user = lib.mkOption {
      type = lib.types.str;
      default = "user";
      description = "The user who owns the ~/pm-stack deployment directory and runs ready-for-agent.";
    };

    tailnetAddress = lib.mkOption {
      type = lib.types.str;
      default = "100.99.113.103";
      description = "The host's tailnet address; Grist and Pad bind it alongside loopback, as the compose files did.";
    };
  };

  config = lib.mkIf cfg.enable {
    virtualisation.oci-containers = {
      backend = "docker";

      containers = {
        pm-postgres = {
          image = "postgres:16-alpine";
          environmentFiles = [ "${pmDir}/.env" ];
          volumes = [ "pm-stack_postgres_data:/var/lib/postgresql/data" ];
        };

        pm-kaneo = {
          image = "ghcr.io/usekaneo/kaneo:2.47.0";
          environmentFiles = [ "${pmDir}/.env" ];
          ports = [ "127.0.0.1:5173:5173" ];
          dependsOn = [ "pm-postgres" ];
        };

        pm-grist = {
          # Grist OSS v1.7.21, pinned by digest as in the compose file.
          image = "gristlabs/grist-oss@sha256:d761de16871ad0022c0ff966b46a92360c23073db8414ebb356cf879f510b1c4";
          environmentFiles = [ "${pmDir}/grist/.env" ];
          environment = {
            GRIST_DEFAULT_EMAIL = "drewrypope@gmail.com";
          };
          ports = [
            "127.0.0.1:8484:8484"
            "${cfg.tailnetAddress}:8484:8484"
          ];
          volumes = [ "grist_grist_data:/persist" ];
        };

        pm-pad = {
          # Pad v0.18.1, tag + digest pin as in the compose file.
          image = "ghcr.io/perpetualsoftware/pad:0.18.1@sha256:45e5a63b941e7562f5e07990224f91ae8019b0185f87c06fc21ab5bec254f13e";
          environmentFiles = [ "${pmDir}/pad/.env" ];
          environment = {
            PAD_PORT = "7777";
            PAD_DB = "/data/pad.db";
          };
          ports = [
            "127.0.0.1:7777:7777"
            "${cfg.tailnetAddress}:7777:7777"
          ];
          volumes = [ "pad_pad_data:/data" ];
          extraOptions = [ "--workdir=/data" ];
        };
      };
    };

    home-manager.users.${cfg.user} = {
      systemd.user.services.ready-for-agent = {
        Unit = {
          Description = "ready-for-agent — label-driven issue dispatch (pm-stack)";
        };

        Service = {
          Type = "simple";
          WorkingDirectory = "${pmDir}/ready-for-agent";
          Environment = [
            "PATH=${home}/.local/bin:/etc/profiles/per-user/user/bin:${home}/.nix-profile/bin:/run/current-system/sw/bin:/usr/bin:/bin"
          ];
          # The binary is installed by pm-stack/ready-for-agent/install.sh;
          # state (SQLite) lives in ~/.local/share/ready-for-agent/ by
          # upstream default. Bare --host binds all interfaces — Drew's
          # 2026-10-25 call (LAN/tailnet fine; no public route exists).
          ExecStart = "${home}/.local/bin/ready-for-agent start --no-open --host";
          Restart = "on-failure";
          RestartSec = 10;
        };

        Install.WantedBy = [ "default.target" ];
      };
    };
  };
}
