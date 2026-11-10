# PM apps (code#1752): the PM-stack servers — Grist, Kaneo +
# PostgreSQL, Pad, and ready-for-agent — declared in this flake like
# everything else on this host, replacing the hand-run deployment
# in ~/pm-stack (docker compose projects plus one hand-written user
# unit) that was not boot-enabled and would not have come back after
# a reboot.
#
# Kaneo + PostgreSQL joined this module on 2026-11-08, completing
# the #1752 unification, under Drew's ruling of 2026-11-08
# (Option A): "nixos for now... unify for all nixos but understand
# we might move all to compose right after. ruling is unify first,
# not making a permanent ruling." PROVISIONAL: this unifies all
# four servers under NixOS; it does not settle compose-vs-NixOS
# permanently.
#
# History (why the sequence matters): this module briefly declared
# oci-containers for Kaneo/PostgreSQL in 2026-10, but that migration
# never completed: the pm-stack compose path stayed enabled and its
# self-heal timer resurrected the compose stack (2026-11-01 00:35),
# after which the two managers fought — docker-pm-kaneo failed at
# every activation (port 5173 already allocated to the compose
# container) and, worse, TWO PostgreSQL postmasters ran against the
# one pm-stack_postgres_data volume from 2026-11-01 00:35 until the
# oci postmaster was stopped later that day (code#3849). The
# duplicate declarations were removed the same day (b19a53ca0).
#
# The 2026-11-08 cutover therefore ran in the opposite order:
#   1. Verified backup first: a pg_dump of the kaneo database,
#      restore-verified into a scratch container with counts
#      identical to live, plus a fresh dump and a cold tar snapshot
#      of the volume taken at the stop (artifacts and hashes are
#      recorded on code#1752).
#   2. The compose units (pm-kaneo.service + pm-kaneo.timer) were
#      removed from the pm-stack module in this same change, and the
#      compose stack was stopped BEFORE the switch — no postmaster
#      overlap was possible at any moment.
#   3. nixos-rebuild switch; verify 8484, 5173, 7777, 6056 answer,
#      exactly one postmaster runs, and Kaneo serves its live data.
#
# Two things the compose setup provided that oci-containers does
# not, reproduced explicitly below:
# - Name resolution: Kaneo finds its database at the host name
#   "postgres" (its upstream default; no DATABASE_URL is set). Under
#   compose, both containers shared the pm-stack_default network,
#   where that name resolves. The 2026-10 oci attempt put both
#   containers on the default bridge with no alias — the oci Kaneo
#   never connected (zero client connections on the oci
#   postmaster). Below, both containers attach to pm-stack_default
#   and pm-postgres carries the network alias "postgres"; the
#   pm-stack-network oneshot creates that network, because
#   `docker compose down` deletes it and nothing else recreates it.
# - deploy.sh is retired as Kaneo's manager. Its compose actions
#   (deploy/restart) must NOT be run anymore — they would resurrect
#   the compose stack against the same volume (the 2026-11-01
#   fight). `deploy.sh render` is still the way to re-render
#   ~/pm-stack/.env and tunnel.env if the SOPS secrets rotate; the
#   rendered files persist on disk and are read as before.
#
# Fidelity rules — this module reproduces the compose setup exactly:
# - Same images, pinned the same way (tag + digest, the digest
#   being the binding pin).
# - Same ports: Grist and Pad on loopback plus the host's tailnet
#   address; Kaneo on loopback 5173; PostgreSQL unpublished, exactly
#   as under compose; ready-for-agent on 6056.
# - Same data, carried IN PLACE: the containers mount the same docker
#   named volumes the compose projects created
#   (grist_grist_data, pad_pad_data, pm-stack_postgres_data), so no
#   data moves and nothing is re-seeded.
# - Same configuration: each container reads the same on-machine
#   .env file its compose project used (~/pm-stack/.env for
#   Kaneo/PostgreSQL, ~/pm-stack/grist/.env, ~/pm-stack/pad/.env —
#   mode 600, rendered by ~/pm-stack's deploy.sh from the pm-stack
#   SOPS file; never hand-edited and never in this repo).
#
# Activation (part of the NixOS switch that lands this module):
#   1. Stop the compose projects, WITHOUT removing volumes:
#        docker compose -f ~/pm-stack/grist/compose.yml down
#        docker compose -f ~/pm-stack/pad/compose.yml down
#   2. nixos-rebuild switch. home-manager renames the hand-written
#      ready-for-agent unit to *.backup, as it did for pm-stack.
#   3. Verify 8484, 7777, and 6056 all answer with the pre-switch
#      data intact.
#   (The Kaneo/PostgreSQL activation of 2026-11-08 followed the
#   cutover steps in the header note above instead.)
#
# Differences from compose, deliberately accepted: compose
# healthchecks have no oci-containers equivalent.
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
    enable = lib.mkEnableOption "the PM app servers (Grist, Kaneo + PostgreSQL, Pad, ready-for-agent) as NixOS-managed services";

    user = lib.mkOption {
      type = lib.types.str;
      default = "user";
      description = "The user who owns the ~/pm-stack deployment directory and runs ready-for-agent.";
    };

    tailnetAddress = lib.mkOption {
      type = lib.types.str;
      default = "100.99.113.103";
      description = "The host's tailnet address; Grist and Pad bind it alongside loopback, as the compose files did, and the ready-for-agent tailnet socket forwards it to the loopback-only app (#1181).";
    };
  };

  config = lib.mkIf cfg.enable {
    virtualisation.oci-containers = {
      backend = "docker";

      containers = {
        pm-postgres = {
          # PostgreSQL 16 (Alpine), tag + digest pin: the digest is
          # the image the compose stack was verified running at the
          # 2026-11-08 cutover, so the cutover changed the manager,
          # not the software.
          image = "postgres:16-alpine@sha256:aa6cb33cf4c19c765cefcf48d96176258800870a9a1e930ad126200f9d825139";
          environmentFiles = [ "${pmDir}/.env" ];
          environment = {
            POSTGRES_USER = "kaneo";
            POSTGRES_DB = "kaneo";
          };
          # The same named volume the compose project created,
          # mounted in place — no data moves.
          volumes = [ "pm-stack_postgres_data:/var/lib/postgresql/data" ];
          # Kaneo resolves its database as the host "postgres" (see
          # the header note): the shared compose network, with this
          # container aliased to that name.
          extraOptions = [
            "--network=pm-stack_default"
            "--network-alias=postgres"
          ];
        };

        pm-kaneo = {
          # Kaneo 2.47.0, tag + digest pin (the image the compose
          # stack was verified running at the cutover, as above).
          image = "ghcr.io/usekaneo/kaneo:2.47.0@sha256:3df62194406061d9a162409b4c08d8af5225e423ce813d509694cea616ac9d23";
          environmentFiles = [ "${pmDir}/.env" ];
          ports = [ "127.0.0.1:5173:5173" ];
          dependsOn = [ "pm-postgres" ];
          extraOptions = [ "--network=pm-stack_default" ];
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

    # The pm-stack_default docker network must exist before
    # docker-pm-postgres / docker-pm-kaneo start (see the header
    # note): compose created it implicitly and `docker compose down`
    # deleted it, so a boot-time oneshot owns it now. Idempotent —
    # a no-op when the network already exists.
    systemd.services.pm-stack-network = {
      description = "Ensure the pm-stack_default docker network exists (pm-apps)";
      after = [ "docker.service" ];
      requires = [ "docker.service" ];
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
      };
      script = ''
        if ! ${config.virtualisation.docker.package}/bin/docker network inspect pm-stack_default >/dev/null 2>&1; then
          ${config.virtualisation.docker.package}/bin/docker network create pm-stack_default
        fi
      '';
    };

    systemd.services.docker-pm-postgres = {
      after = [ "pm-stack-network.service" ];
      requires = [ "pm-stack-network.service" ];
    };

    systemd.services.docker-pm-kaneo = {
      after = [ "pm-stack-network.service" ];
      requires = [ "pm-stack-network.service" ];
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
          # upstream default. No --host flag: upstream's default bind is
          # loopback only. The all-interfaces bind (bare --host) was
          # retired by #1181 — it also claimed any future WAN-facing
          # interface, broader than the stack's access model (loopback +
          # tailnet; public only behind a password). Tailnet access is
          # restored by the ready-for-agent-tailnet socket below, which
          # forwards the tailnet address to the loopback listener.
          ExecStart = "${home}/.local/bin/ready-for-agent start --no-open";
          Restart = "on-failure";
          RestartSec = 10;
        };

        Install.WantedBy = [ "default.target" ];
      };

      # Tailnet entry point for ready-for-agent (#1181): the app itself
      # binds loopback only (single-address --host upstream), so the
      # tailnet address is served by a socket-activated systemd proxy
      # forwarding to 127.0.0.1:6056 — the same loopback + tailnet scope
      # Grist and Pad get from their dual port publishes.
      systemd.user.sockets.ready-for-agent-tailnet = {
        Socket = {
          ListenStream = "${cfg.tailnetAddress}:6056";
        };

        Install.WantedBy = [ "sockets.target" ];
      };

      systemd.user.services.ready-for-agent-tailnet = {
        Unit = {
          Description = "ready-for-agent tailnet forward (socket-activated proxy to loopback)";
          Requires = [ "ready-for-agent.service" ];
          After = [ "ready-for-agent.service" ];
        };

        Service = {
          ExecStart = "${config.systemd.package}/lib/systemd/systemd-socket-proxyd 127.0.0.1:6056";
        };
      };
    };
  };
}
