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
  # Local service ports, repeated here because OpenClaw's config is
  # rendered as data, not shell env. (The provider-proxy ports that
  # once had to stay in sync with home/common/default.nix are all
  # retired — the last, museProxyPort 18747, on 2026-11-06.)
  # The opencode-route provider talks to the OpenCode server facade
  # directly (127.0.0.1:13420/v1) since the route proxy's retirement
  # (opencode#1153, 2026-11-04); the proxy's port (18750) is free.
  opencodeServerPort = 13420;
  # Fixed loopback port of the dedicated embedding server below (#1300).
  embeddingPort = 18791;
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
  # acpx's default `npx -y ...` upstream downloads: opencode runs through
  # the ACP-to-HTTP adapter in this repo (bin/opencode-acp-adapter.mjs,
  # #1243) so OpenClaw sessions land on the standing shared
  # `opencode serve` at 127.0.0.1:13420 instead of a private per-process
  # server; codex runs the local wrapper in ~/.openclaw/acpx
  # (bundled adapter, no npx fetch); agy is the system
  # antigravity-acp server. There is deliberately NO claude agent
  # here: Drew's order 2026-11-08 - nothing automated may spawn the
  # real Claude Code CLI (code#7098); its old wrapper path is now a
  # fail-closed stub.
  # FALLBACK (#1243): to revert opencode to the private-server path,
  # restore opencode.command below (and agents.opencode in the embedded
  # acpx override) to "/etc/profiles/per-user/user/bin/opencode acp".
  home.file.".acpx/config.json".text = builtins.toJSON {
    agents = {
      opencode.command = "/home/user/code/bin/opencode-acp-adapter.mjs --server http://127.0.0.1:13420";
      codex.command = "/home/user/.openclaw/acpx/codex-acp-wrapper.mjs";
      agy.command = "/run/current-system/sw/bin/agy_acp_server";
    };
  };

  # Start after the services it depends on. The gateway's model lifeline
  # is the OpenCode server (opencode.service, facade on 127.0.0.1:13420)
  # since the route proxy retired (opencode#1153, 2026-11-04); it is
  # ordered After it but does not Want it (the server has its own
  # enablement; the ExecStartPre below waits for its service record).
  # The gateway must not Want the per-proxy units: a gateway restart
  # would re-pull them (the manual commandcode-proxy unit once
  # crash-looped on its already-taken port, #883, in the retired
  # proxy-supervisor era). The commandcode-proxy and agy-proxy units
  # themselves are gone — commandcode in the 2026-11-06 proxy-infra
  # decommission, agy on 2026-10-30 — so their After entries below
  # are gone too. Only mcpx and the embedding server are Wanted.
  # Stable local embedding endpoint for memory-lancedb (#1300 phase 1):
  # a dedicated llama-server on a FIXED loopback port, reusing the
  # llama.cpp build and the Qwen3-Embedding-0.6B GGUF that OpenCode's
  # messaging embeddings already keep in ~/.cache/opencode (OpenCode's
  # own per-process embedding servers use dynamic ports, which a static
  # gateway config cannot point at). If that cache is ever cleared, the
  # ConditionPathExists below fails this unit loudly instead of the
  # gateway silently losing vector recall.
  systemd.user.services.openclaw-embedding = {
    Unit = {
      Description = "Local embedding server for OpenClaw memory (llama.cpp)";
      ConditionPathExists = "${homeDir}/.cache/opencode/models/Qwen3-Embedding-0.6B-Q8_0.gguf";
    };
    Service = {
      ExecStart = "${homeDir}/.cache/opencode/bin/llama.cpp/b11669/llama-b11669/llama-server --embedding --model ${homeDir}/.cache/opencode/models/Qwen3-Embedding-0.6B-Q8_0.gguf --host 127.0.0.1 --port ${toString embeddingPort}";
      Restart = "on-failure";
      RestartSec = 5;
    };
    Install.WantedBy = [ "default.target" ];
  };

  systemd.user.services.openclaw-gateway = {
    Unit = {
      After = [
        "openclaw-embedding.service"
        "mcpx.service"
        "opencode.service"
      ];
      Wants = [
        "openclaw-embedding.service"
        "mcpx.service"
      ];
      # The 2026-11-04 death: a restart raced the OpenCode server's
      # service record, ExecStartPre failed five times inside the
      # default 10s start-limit window, and systemd parked the unit as
      # start-limit-hit — down for a day, silently. A transient
      # pre-start failure must never be able to do that again: wide
      # window, generous burst, slower restart spacing.
      StartLimitIntervalSec = "600";
      StartLimitBurst = 30;
    };
    Service = {
      RestartSec = lib.mkForce "5s"; # module default is 1s — the burst spacing that let 2026-11-04 happen
      # Materialize the facade credential at every gateway start
      # (opencode#1146 step 3): the opencode server password lives
      # in its service record — the same source oc-password-sync
      # reads — and the provider config references the value below
      # as an env SecretRef (programs.openclaw.environment reads
      # the file at start). Rebuilt per start, so a server-side
      # password rotation heals at the next gateway restart. The
      # file holds the complete Authorization header value.
      ExecStartPre = toString (
        pkgs.writeShellScript "openclaw-facade-auth-materialize" ''
          set -eu
          umask 077
          mkdir -p "$HOME/.local/state/openclaw"
          # The OpenCode server writes its service record at ITS start;
          # a gateway restart that races it (both restarted by one
          # system switch, 2026-11-04) must wait for the record instead
          # of failing instantly: that night the record was missing for
          # ~3 minutes, this step failed five times in seconds, the
          # unit hit the start limit, and the gateway stayed down until
          # manually revived a day later. Wait up to ~3 minutes for a
          # record that yields a password, then fail loudly.
          record="$HOME/.local/state/opencode/service-prod.json"
          PW=""
          for attempt in $(seq 1 90); do
            if [ -f "$record" ]; then
              PW=$(${pkgs.jq}/bin/jq -r '.password // empty' "$record" 2>/dev/null || true)
              [ -n "$PW" ] && break
            fi
            sleep 2
          done
          if [ -z "$PW" ]; then
            echo "openclaw-facade-auth-materialize: $record never yielded a password" >&2
            exit 1
          fi
          printf 'Basic %s' "$(printf 'opencode:%s' "$PW" | ${pkgs.coreutils}/bin/base64 -w0)" > "$HOME/.local/state/openclaw/opencode-server-auth"
        ''
      );
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
      OPENCLAW_TELEGRAM_BOT_TOKEN = "/run/secrets/openclaw/telegram-bot-token";
      TAVILY_API_KEY = "/run/secrets/openclaw/tavily-api-key";
      OPENCODE_SERVER_AUTH = "${homeDir}/.local/state/openclaw/opencode-server-auth";
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
      # NOTE: no "telegram" entry here — in the pinned build (2026.10.7)
      # Telegram is a bundled core channel: extensions/telegram ships
      # inside the openclaw-gateway package itself, so it is absent from
      # the external runtime-plugin catalog (report.json) and listing it
      # here fails evaluation (#7042). It activates from the
      # channels.telegram config + plugins.entries.telegram below
      # (bot @shed_openclaw_bot, created 2026-11-07; token via env
      # SecretRef).
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
      "memory-lancedb" # LanceDB long-term memory; owns plugins.slots.memory (#1300 phase 1)
      # memory-wiki is not in the pinned nix-openclaw runtime catalog yet; enable it once packaged.
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

          - All new code goes in **developing-today/code** (this laptop's NixOS flake is its root, ~/code). Other fleet repos: developing-today-forks/opencode, developing-today-forks/openchamber, dezren39/nix.
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
          - No provider proxies remain on loopback: the last one, muse 18747, retired 2026-11-06 when muse-direct went native in the fork (the codex, clef, agy, antigravity-ls, design-code, commandcode and claude-code doors retired earlier — codex, commandcode and claude-code in the 2026-11-06 proxy-infra decommission); the OpenCode server facade on 13420 is this gateway's only model route. This gateway: 18789 (tailnet).
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
        # Telegram wired 2026-11-07: bot @shed_openclaw_bot (Shed),
        # created via BotFather from Drew's account; token via env
        # SecretRef fed from the sops-decrypted laptop file. DMs from
        # Drew only (numeric user id).
        telegram = {
          enabled = true;
          botToken = {
            source = "env";
            provider = "default";
            id = "OPENCLAW_TELEGRAM_BOT_TOKEN";
          };
          dmPolicy = "allowlist";
          allowFrom = [ "8290042325" ]; # Drew
          groupPolicy = "allowlist";
        };
      };

      # Model routing: OpenClaw uses only the opencode-route pools
      # (Drew, 2026-10-31), reached through the OpenCode server
      # facade since the route proxy retired (opencode#1153); the
      # underlying pools are reached exclusively through OpenCode
      # routes (see QUOTAS.md).
      models = {
        mode = "merge";
        providers = {
          # The OpenCode server facade fronts the route pools
          # (#2518): each model id is an opencode-route pool. The
          # facade requires the server credential, carried as an
          # Authorization header whose value is an env SecretRef —
          # materialized at gateway start by the unit's ExecStartPre
          # from the server's service record, so the credential
          # itself never enters the store (opencode#1146 step 3).
          opencode-route = {
            baseUrl = "http://127.0.0.1:${toString opencodeServerPort}/v1";
            apiKey = "local-proxy";
            api = "openai-completions";
            headers.Authorization = {
              source = "env";
              provider = "default";
              id = "OPENCODE_SERVER_AUTH";
            };
            models = [
              (model "workhorse" "Workhorse (OpenCode route pool)")
              (model "speed" "Speed (OpenCode route pool)")
              (model "specialist" "Specialist (OpenCode route pool)")
              (model "summit" "Summit (OpenCode route pool)")
              (model "scout" "Scout (OpenCode route pool)")
            ];
          };
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
            # NOTE: heartbeat.skipWhenBusy (upstream-recommended on
            # constrained hosts) cannot be expressed yet — the pinned
            # nix-openclaw generated schema has no such option. Revisit
            # after the nix-openclaw upgrade.
            activeHours = {
              start = "07:00";
              end = "23:00";
            };
          };
        };
        ownership = "explicit";
        entries = {
          main = {
            name = "Shed";
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
        # npmjs route; 120s check timeout, 2026-10-26). codex runs
        # the local wrapper in ~/.openclaw/acpx (adapter bundled with
        # the acpx plugin; the store repoint is code#1711); agy runs the
        # system antigravity-acp binary. Drew enabled all four
        # 2026-10-28; claude was REMOVED 2026-11-08 by Drew's order -
        # the real Claude Code CLI must never be spawned headlessly
        # (code#7098: a headless CLI run blanked
        # ~/.claude/.credentials.json on 2026-11-07). Codex ALSO works
        # via its native runtime
        # plugins for chat bindings — ACP is the background-spawn path.
        allowedAgents = [
          "opencode"
          "codex"
          "agy"
        ];
        dispatch.enabled = true;
      };

      plugins = {
        # memory-lancedb owns the memory slot (#1300 phase 1, approved
        # by Drew 2026-10-26). Reversible: remove this slot line (and
        # the runtime plugin above) to fall back to the built-in
        # memory backend.
        slots.memory = "memory-lancedb";

        entries = {
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
              # Via the ACP-to-HTTP adapter (#1243), not `opencode acp`:
              # sessions land on the shared serve at 127.0.0.1:13420.
              # Fallback: command "/etc/profiles/per-user/user/bin/opencode"
              # with args [ "acp" ] restores the private-server path.
              agents.opencode = {
                command = "${homeDir}/code/bin/opencode-acp-adapter.mjs";
                args = [
                  "--server"
                  "http://127.0.0.1:13420"
                ];
              };
              agents.codex = {
                command = "/home/user/.openclaw/acpx/codex-acp-wrapper.mjs";
              };
              agents.agy = {
                command = "/run/current-system/sw/bin/agy_acp_server";
              };
            };
          };
          slack.enabled = true;
          telegram.enabled = true;
          codex.enabled = true;
          copilot.enabled = true;
          "memory-lancedb" = {
            enabled = true;
            # Conversation access for the plugin's typed hooks. Without
            # this, the gateway blocks memory-lancedb's before_prompt_build
            # (autoRecall injection) and agent_end (autoCapture) hooks —
            # observed live 2026-11-05: the plugin initialized against an
            # empty LanceDB (0 memories) while both hooks were blocked,
            # so the memory slot has been decorative since the Nov 4
            # deployment. This flag is upstream's per-plugin consent for
            # exactly that access.
            hooks.allowConversationAccess = true;
            config = {
              embedding = {
                # Direct OpenAI-compatible path against the fixed local
                # llama-server above. The apiKey is a placeholder label,
                # not a credential: the loopback server checks no key
                # (same posture as localProvider).
                apiKey = "local-embedding";
                baseUrl = "http://127.0.0.1:${toString embeddingPort}/v1";
                model = "Qwen3-Embedding-0.6B-Q8_0";
                dimensions = 1024; # Qwen3-Embedding-0.6B hidden size
              };
              autoRecall = true;
              # Auto-capture ON (2026-11-05): phase 1 (#1300) shipped
              # with capture off while recall proved out; the standard
              # advanced lancedb setup runs both, and Shed's memory
              # only compounds if Drew's messages are captured as they
              # land. The plugin filters (length, markup) still apply.
              autoCapture = true;
            };
          };
        };
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
