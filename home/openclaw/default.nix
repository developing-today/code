{
  lib,
  inputs,
  pkgs,
  system,
  ...
}:
# OpenClaw (https://github.com/openclaw/openclaw) on this laptop, packaged
# declaratively via nix-openclaw (github:openclaw/nix-openclaw, pinned in
# flake.lock). Home Manager module -> systemd USER service
# `openclaw-gateway`, WantedBy default.target; the user has linger enabled,
# so it starts at boot with no login required.
#
# Role in the fleet: a peer intelligence source next to Muse (Archer) and
# Aster (Dot). Its coding arm is OpenCode — the same engine, config, auth
# and route pools OpenChamber drives — reached over ACP (agent id
# `opencode`). See workspace/FLEET.md and workspace/QUOTAS.md (materialized
# into ~/.openclaw/workspace) for the knowledge pack it runs with.
#
# Secrets: referenced by path/name only, never values. The gateway token
# lives in /run/secrets/openclaw/gateway-token (decrypted by sops-nix at
# activation from secrets/sops/common/bws.yaml; never in git or the Nix
# store) and is read at service start.
let
  homeDir = "/home/user";
  acpxPlugin = inputs.nix-openclaw.packages.${system}.openclaw-runtime-plugin-acpx;
  # Local provider-proxy ports. The canonical definitions live in
  # home/common/default.nix (clefProxyPort..claudeCodeProxyPort); these are
  # the same numbers, repeated here because OpenClaw's config is rendered
  # as data, not shell env. Keep in sync with home/common.
  opencodeRouteProxyPort = 18750;
  localProvider = port: models: {
    baseUrl = "http://127.0.0.1:${toString port}/v1";
    # The laptop proxies are loopback-only and do not check an API key;
    # this is a placeholder label, not a credential.
    apiKey = "local-proxy";
    api = "openai-completions";
    inherit models;
  };
  model = id: name: { inherit id name; };
in
{
  imports = [ inputs.nix-openclaw.homeManagerModules.openclaw ];

  # Point acpx's harnesses at this laptop's pinned binaries instead of
  # acpx's default `npx -y ...` upstream downloads: opencode is the pinned
  # OpenCode fork (the build OpenChamber uses); claude/codex run the local
  # managed wrappers in ~/.openclaw/acpx (bundled adapters, no npx fetch); agy is
  # the system antigravity-acp server.
  # Keep separate names from OpenClaw's generated wrappers: plugin admission
  # can rewrite those, and they remain a rollback path until verified.
  home.file = {
    ".openclaw/acpx/bin/claude-agent-acp.mjs".source = "${acpxPlugin}/node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js";
    ".openclaw/acpx/bin/codex-acp.mjs".source = "${acpxPlugin}/node_modules/@agentclientprotocol/codex-acp/dist/index.js";
    ".openclaw/acpx/managed-claude-agent-acp-wrapper.mjs".source = ./acpx/claude-agent-acp-wrapper.mjs;
    ".openclaw/acpx/managed-codex-acp-wrapper.mjs".source = ./acpx/codex-acp-wrapper.mjs;
  };
  home.file.".acpx/config.json".text = builtins.toJSON {
    agents = {
      opencode.command = "/etc/profiles/per-user/user/bin/opencode acp";
      claude.command = "/home/user/.openclaw/acpx/managed-claude-agent-acp-wrapper.mjs";
      codex.command = "/home/user/.openclaw/acpx/managed-codex-acp-wrapper.mjs";
      agy.command = "/run/current-system/sw/bin/agy_acp_server";
    };
  };

  # Start after the provider proxies + mcpx it depends on. The proxies
  # are owned by the proxy-supervisor (transient fps-* units on the
  # canonical ports), so the gateway must not Want the manual per-proxy
  # units: a gateway restart would re-pull them, and the manual
  # commandcode-proxy unit crash-loops on its already-taken port (#883).
  # The gateway reaches the proxies by port, not by unit; only mcpx and
  # the opencode-route proxy are real unit dependencies.
  systemd.user.services.openclaw-gateway = {
    Unit = {
      After = [
        "opencode-route-proxy.service"
        "commandcode-proxy.service"
        "design-code-proxy.service"
        "agy-proxy.service"
        "codex-proxy.service"
        "mcpx.service"
      ];
      Wants = [
        "opencode-route-proxy.service"
        "mcpx.service"
      ];
    };
  };

  programs.openclaw = {
    enable = true;
    systemd.enable = true;

    # Gateway runtime env: a value that names an existing file is read
    # from that file at service start (nix-openclaw runtime contract), so
    # the token itself never enters the store.
    environment = {
      OPENCLAW_GATEWAY_TOKEN = "/run/secrets/openclaw/gateway-token";
      OPENCLAW_SLACK_BOT_TOKEN = "/run/secrets/openclaw/slack-bot-token";
      OPENCLAW_SLACK_APP_TOKEN = "/run/secrets/openclaw/slack-app-token";
    };

    # Extra binaries on the gateway's PATH (not the login shell's):
    # the fleet OpenCode fork (ACP/CLI work), gh for repo operations.
    runtimePackages = [
      inputs.opencode.packages.${system}.opencode
      pkgs.gh
    ];

    workspace = {
      bootstrapFiles = {
        agents = ./workspace/AGENTS.md;
        soul = ./workspace/SOUL.md;
        tools = ./workspace/TOOLS.md;
        identity = ./workspace/IDENTITY.md;
        user = ./workspace/USER.md;
        heartbeat = ./workspace/HEARTBEAT.md;
      };
      files = {
        "FLEET.md" = ./workspace/FLEET.md;
        "QUOTAS.md" = ./workspace/QUOTAS.md;
        "CHANNELS.md" = ./workspace/CHANNELS.md;
      };
    };

    # nix-openclaw tool plugins (CLI tools + skills, packaged in Nix).
    bundledPlugins = {
      # summarize is vendored as a custom plugin below (see
      # plugins/summarize/flake.nix): the bundled plugin's pinned pnpm
      # deps hash does not reproduce under the toolchain that
      # nix-openclaw's hardcoded tools rev builds with, which broke the
      # 2026-10-30 switch (hash mismatch in summarize-pnpm-deps).
      # Do not re-enable until upstream re-pins.
      summarize.enable = false; # summarize URLs / PDFs / videos (vendored below)
      gogcli.enable = true; # Google Workspace CLI (gog)
      goplaces.enable = true; # Google Places CLI
      # peekaboo / poltergeist are macOS-only in the nix-openclaw
      # catalog (linux = false) and hard-fail evaluation on Linux.
      qmd.enable = true; # local markdown knowledge search
      sag.enable = true; # text-to-speech
      camsnap.enable = true; # camera snapshots
      discrawl.enable = true; # Discord archive/search
      wacrawl.enable = true; # WhatsApp archive/search
    };

    # Custom plugins supplied as plugin flakes
    # (programs.openclaw.customPlugins).
    customPlugins = [
      {
        # nix-openclaw runs builtins.getFlake on this string. A plain
        # source-tree path is an UNLOCKED flake reference and pure
        # evaluation (CI host-eval, #705) refuses it; a root-flake path
        # input is no fix either — its lock node inherits the ROOT
        # sourceInfo narHash, which does not match the plugin subtree
        # the path fetcher hashes (Codex P1 on PR #3025). So fetch the
        # subtree as a locked tree of its own: fetchTree with the
        # subtree's own narHash pinned here, then hand getFlake the
        # resulting store path with that same hash.
        # When the plugin's files change, evaluation fails on the
        # hash mismatch until the pin is refreshed with:
        #   nix eval --impure --raw --expr '(builtins.fetchTree { type = "path"; path = ./home/openclaw/plugins/summarize; }).narHash'
        # unsafeDiscardStringContext: getFlake also rejects strings
        # carrying store-path context.
        source =
          let
            pluginTree = builtins.fetchTree {
              type = "path";
              path = ./plugins/summarize;
              narHash = "sha256-y4xkNIopeCdJ/C/PiIZeKlfALrmxUrBYg5CXUSqsMXU=";
            };
          in
          builtins.unsafeDiscardStringContext "path:${pluginTree.outPath}?narHash=${pluginTree.narHash}";
      }
    ];

    # OpenClaw runtime plugins (channels, harnesses, providers, search),
    # packaged immutably by nix-openclaw from the pinned upstream catalog.
    runtimePlugins = [
      "acpx" # ACP backend: opencode / claude / codex / gemini / copilot harnesses
      "slack" # Slack channel (channel inert until app tokens are wired; see CHANNELS.md)
      "discord"
      "whatsapp"
      "matrix"
      "signal"
      "codex" # native Codex app-server harness
      "copilot" # GitHub Copilot agent runtime
      "opencode" # OpenCode Zen/Go provider
      "deepseek" # DeepSeek provider (plan routes are via the local proxies below)
      "meta" # Meta provider
      "brave"
      "tavily"
      "exa"
      "firecrawl"
      "perplexity"
      "duckduckgo"
      "searxng" # keyless web search (added 2026-10-26, Drew: more plugins)
      "lobster" # typed workflow runtime for resumable automations
      "diffs" # diff rendering for code review in chat
      "tokenjuice" # context compression
    ];

    # Instance-local skills that encode how this fleet works.
    skills = [
      {
        name = "fleet-intelligence-protocol";
        description = "Who Muse/Archer, Aster/Dot, OpenChamber and OpenClaw are, and how to work with each.";
        mode = "inline";
        body = ''
          # Fleet intelligence protocol

          - **Muse (Archer)** is Drew's chief of staff: cross-account memory (Gmail/Calendar/Drive), coordination, follow-through. Work that needs his connected accounts or long-term memory goes to Drew for Muse — do not fake it locally.
          - **Aster (Dot)** is the second-opinion partner in the Developing Today Slack (#alignment). Bounce design/analysis questions there; chat is unmetered, Aster Work/Codex tasks are not — never trigger those when an OpenCode session can do the job.
          - **OpenChamber** is the orchestration GUI over OpenCode sessions on this laptop. For complicated work, OpenChamber/OpenCode comes first.
          - **You (OpenClaw)** are the always-on local node: channels, monitoring, quick work, and delegation to OpenCode over ACP.

          Full detail: workspace FLEET.md.
        '';
      }
      {
        name = "quota-discipline";
        description = "Plan-quota rules for every model call: plans only, burn by expiry, OpenAI is the final reserve.";
        mode = "inline";
        body = ''
          # Quota discipline (binding)

          - Plans only, never funds/PAYG. A 402 or quota error is a wall: one retry max on the same pool, then rotate or stop and report.
          - Burn by expiry: use the pool that expires soonest; OpenAI remains the final reserve but is reachable only as a leaf inside the OpenCode route pools, not as a direct OpenClaw provider.
          - Choose model AND effort per task, deliberately. Routine work: DeepSeek Flash. Flagships only for genuinely hard problems, right-sized effort.
          - Measure quotas before and during heavy work where a checker exists.

          Full detail: workspace QUOTAS.md.
        '';
      }
      {
        name = "repo-fleet";
        description = "The four fleet repos, where code goes, and the git/merge discipline.";
        mode = "inline";
        body = ''
          # Repo fleet

          - All new code goes in **developing-today/code** (this laptop's NixOS flake is its root, ~/code). Other fleet repos: developing-today/opencode, developing-today/openchamber, dezren39/nix.
          - Work in worktrees on branches; review `git status` before every commit; surgical staging; merge commits only; never force-push; never `git stash`.
          - Finished + verified work is merged (by you), never parked in drafts. Blocked work gets an issue plus a named next action.
          - Never touch the `vertexinc` org.
          - ROADMAP files keep Now/Next only; GitHub is the record of done work.
        '';
      }
      {
        name = "laptop-operations";
        description = "This laptop's services, proxy ports, and how to check/restart them.";
        mode = "inline";
        body = ''
          # Laptop operations

          - Everything is a systemd **user** service (linger on): `systemctl --user status|restart <name>`, logs via `journalctl --user -u <name>`.
          - Provider proxies on loopback: clef 18742, agy 18743, codex 18744, antigravity-ls 18745, design-code 18746, muse 18747, commandcode 18748, claude-code 18749, opencode-route 18750. This gateway: 18789 (tailnet).
          - OpenChamber's managed OpenCode port is dynamic — read `~/.config/openchamber/managed-opencode/state.json`, never hardcode it.
          - Secrets are referenced by name only (Bitwarden Secrets Manager; sops files under /run/secrets/agent-auth/). Never print or commit a secret value.

          Full detail: workspace TOOLS.md.
        '';
      }
    ];

    config = {
      update.checkOnStart = false;

      gateway = {
        mode = "local";
        bind = "tailnet";
        port = 18789;
        auth = {
          mode = "token";
          token = {
            source = "env";
            provider = "default";
            id = "OPENCLAW_GATEWAY_TOKEN";
          };
        };
      };

      # Chat channels. Slack wired 2026-10-26: app A0CD7L3S81W, bot
      # @openclaw (U0CC72H7KA7); tokens via env SecretRefs fed from
      # laptop files (interim home for sops-managed secrets). DMs from
      # Drew only; channel access gets allowlisted per channel later.
      channels = {
        slack = {
          enabled = true;
          mode = "socket";
          botToken = {
            source = "env";
            provider = "default";
            id = "OPENCLAW_SLACK_BOT_TOKEN";
          };
          appToken = {
            source = "env";
            provider = "default";
            id = "OPENCLAW_SLACK_APP_TOKEN";
          };
          dmPolicy = "allowlist";
          allowFrom = [ "U0CBX163G8Z" ]; # Drew
          groupPolicy = "allowlist";
        };
      };

      # Model routing: OpenClaw uses only the opencode-route door
      # (Drew, 2026-10-31). That door is pass-through only — it handles
      # auth and translates pool routes; the underlying pools are
      # reached exclusively through OpenCode routes (see QUOTAS.md).
      models = {
        mode = "merge";
        providers = {
          # The route proxy (code#2229) fronts the OpenCode route
          # pools (#2518): each model id is an opencode-route pool.
          opencode-route = localProvider opencodeRouteProxyPort [
            (model "workhorse" "Workhorse (OpenCode route pool)")
            (model "speed" "Speed (OpenCode route pool)")
            (model "specialist" "Specialist (OpenCode route pool)")
            (model "summit" "Summit (OpenCode route pool)")
            (model "scout" "Scout (OpenCode route pool)")
          ];
        };
      };

      # One DM session per channel+peer, so a future second channel
      # never shares a transcript with Slack DMs.
      session = {
        dmScope = "per-channel-peer";
      };

      agents = {
        defaults = {
          model = {
            primary = "opencode-route/workhorse";
            fallbacks = [
              "opencode-route/specialist"
              "opencode-route/speed"
            ];
          };
          heartbeat = {
            every = "30m";
            lightContext = true;
            isolatedSession = true;
            activeHours = {
              start = "07:00";
              end = "23:00";
            };
          };
        };
        ownership = "explicit";
        entries = {
          main = {
            name = "OpenClaw (laptop node)";
          };
          # The ACP-runtime agent entry (opencode-acp, #2229 mechanism 1)
          # is removed for now: the nix-openclaw generated schema types an
          # entry's runtime as a oneOf of two submodules, and under the
          # pinned nixpkgs the acp branch is unreachable, so any entry
          # using it fails the whole configuration evaluation. Full
          # analysis and the minimal reproduction: code#2509. Restore the
          # entry (tracked by #2256) once the schema can express it. The
          # ACP capability itself (acp block below: enabled, allowedAgents,
          # acpx plugin) is unaffected.
        };
      };

      # ACP: OpenClaw may spawn the fleet's coding harnesses. Default
      # target is OpenCode — the OpenChamber engine.
      acp = {
        enabled = true;
        backend = "acpx";
        defaultAgent = "opencode";
        # Every allowed agent gets a per-agent command override below,
        # so the acpx startup check never resolves a harness to an
        # npx-fetched adapter (that fetch hangs on this laptop's flaky
        # npmjs route; 120s check timeout, 2026-10-26). claude/codex run
        # the managed wrappers in ~/.openclaw/acpx (adapters bundled with
        # the acpx plugin); agy runs the
        # system antigravity-acp binary. Drew enabled all four
        # 2026-10-28. Claude/Codex ALSO work via their native runtime
        # plugins for chat bindings — ACP is the background-spawn path.
        allowedAgents = [
          "opencode"
          "claude"
          "codex"
          "agy"
        ];
        dispatch.enabled = true;
      };

      plugins.entries = {
        acpx = {
          enabled = true;
          config = {
            # ACP sessions are non-interactive; on Drew's own repos the
            # fleet posture is approve-all, with unexpected prompts denied
            # rather than hanging the session.
            permissionMode = "approve-all";
            nonInteractivePermissions = "deny";
            # Per-agent command override for the EMBEDDED acpx runtime.
            # Without this, acpx defaults to `npx -y opencode-ai acp` —
            # upstream OpenCode v1, which rejects the fleet's v2 config
            # (and the npx fetch hangs on this laptop's npmjs route).
            # The standalone acpx CLI reads ~/.acpx/config.json instead;
            # that file is kept in sync above, but the plugin ignores it.
            agents.opencode = {
              command = "/etc/profiles/per-user/user/bin/opencode";
              args = [ "acp" ];
            };
            agents.claude = {
              command = "/home/user/.openclaw/acpx/managed-claude-agent-acp-wrapper.mjs";
            };
            agents.codex = {
              command = "/home/user/.openclaw/acpx/managed-codex-acp-wrapper.mjs";
            };
            agents.agy = {
              command = "/run/current-system/sw/bin/agy_acp_server";
            };
          };
        };
        slack.enabled = true;
        codex.enabled = true;
        copilot.enabled = true;
      };

      # Direct MCP wiring to the fleet's most useful servers (the same
      # set mcpx fronts; see ~/.config/mcpx/config.json).
      mcp.servers = {
        codedb = {
          command = "${homeDir}/bin/codedb";
          args = [ "mcp" ];
          env.CODEDB_NO_TELEMETRY = "1";
        };
        fff = {
          command = "${homeDir}/.local/bin/fff-mcp";
          args = [ ];
        };
        context7 = {
          command = "npx";
          args = [
            "-y"
            "mcp-remote"
            "https://mcp.context7.com/mcp"
          ];
        };
        desktop-commander = {
          command = "npx";
          args = [
            "-y"
            "@wonderwhy-er/desktop-commander"
          ];
        };
      };

      cron.enabled = true;
    };
  };
}
