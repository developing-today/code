# The mcpx daemon's configuration (services.mcpx.settings in
# home/common/default.nix), as a pure data file.
#
# This is the former ~/code/.mcpx.json — the daemon's hand-maintained
# config, which ~/.config/mcpx/config.json used to link to — moved
# into the Nix configuration with the 2026-11-03 cascade migration.
# The mcpx module renders it to the store and starts the daemon with
# --config pointing there; ~/.config/mcpx/config.json is now a link
# to the rendered file. To change the daemon's servers, edit this
# file and switch — editing a JSON file in the checkout no longer
# reaches the daemon.
{
  "$schema" = "https://raw.githubusercontent.com/dezren39/mcpx/main/schema/mcpx.schema.json";
  # Stable loopback endpoint held by mcpx.socket; the Go daemon adopts
  # systemd's fd 3 when LISTEN_FDS is present (#1822, #12276).
  "daemon" = {
    "port" = 34005;
  };
  "mcpServers" = {
    "atlas" = {
      # Atlas search (#8625): the atlas-search Worker's MCP
      # endpoint (pkgs/atlas-search/worker, POST /mcp). The gate
      # token is expanded from the daemon's environment at
      # runtime (see services.mcpx.environmentFile in
      # home/common/default.nix); the value never lands here or
      # in the Nix store.
      url = "https://atlas-search.developing-today.workers.dev/mcp";
      auth = {
        type = "bearer";
        token = "\${ATLAS_SEARCH_GATE_TOKEN}";
      };
    };
    "antigravity" = {
      "args" = [
        "-y"
        "@ask-llm/antigravity-mcp"
      ];
      "command" = "npx";
    };
    "chrome-devtools" = {
      "args" = [
        "-y"
        "chrome-devtools-mcp@latest"
        "--executable-path=/run/current-system/sw/bin/chromium"
        "--headless"
        "--isolated"
        "--viewport=800x600"
        "--no-usage-statistics"
        "--no-performance-crux"
        "--no-category-emulation"
      ];
      "command" = "npx";
      "mcpx" = {
        "scope" = "session";
        "sharing" = "exclusive";
      };
    };
    "cloudrun" = {
      "args" = [
        "-y"
        "@google-cloud/cloud-run-mcp"
      ];
      "command" = "npx";
    };
    "codebase-memory" = {
      "args" = [
        "-y"
        "codebase-memory-mcp"
      ];
      "command" = "npx";
    };
    "codedb" = {
      "args" = [ "/home/user/git/code/bin/codedb-bridge.mjs" ];
      "command" = "node";
      "env" = {
        "CODEDB_DAEMON_URL" = "http://127.0.0.1:7719";
        "CODEDB_NO_TELEMETRY" = "1";
      };
    };
    "codex-connectors" = {
      "args" = [
        "-y"
        "@wileai/codex-connectors-mcp@0.1.1"
      ];
      "command" = "npx";
      "env" = {
        "CODEX_CONNECTORS_ALLOW" =
          "Airtable, ChatGPT Space, Codex Document Control, Context.dev, Context7, DigitalOcean, Dropbox, Engram, Exa, Firecrawl, GitHub, Gmail, Google Calendar, Google Drive, HAPI MCP Registry, Hotline, Lovable, Neon, OpenAI Platform, Parallel Search, Pets, Plugin Management, Railway, Remote Desktop Commander, Replit, Safety Settings, Search Service, Sites, Slack, Sprites, Stack Overflow For Agents, Supabase, tldraw, Todoist: To Do List & Calendar, Vercel, Walmart, Wolfram";
        "CODEX_CONNECTORS_MODE" = "compact";
      };
    };
    "context7" = {
      "args" = [
        "-y"
        "mcp-remote"
        "https://mcp.context7.com/mcp"
      ];
      "command" = "npx";
    };
    "deepwiki" = {
      "args" = [
        "-y"
        "mcp-remote"
        "https://mcp.deepwiki.com/mcp"
      ];
      "command" = "npx";
    };
    "desktop-commander" = {
      "args" = [
        "-y"
        "@wonderwhy-er/desktop-commander"
      ];
      "command" = "npx";
    };
    # readOnlyTools (#6116): fff-mcp publishes no tool annotations, so
    # the specification's defaults made every search a possibly
    # destructive write and the write-policy gate (default ask)
    # refused them all in headless/API sessions, where the
    # confirmation question has no answerer. All three fff tools are
    # searches; the operator declares them read-only here.
    "fff" = {
      "args" = [ ];
      "command" = "fff-mcp";
      "mcpx" = {
        "readOnlyTools" = [
          "find_files"
          "grep"
          "multi_grep"
        ];
      };
    };
    "fff-nix" = {
      "args" = [ "/home/user/code" ];
      "command" = "fff-mcp";
      "mcpx" = {
        "readOnlyTools" = [
          "find_files"
          "grep"
          "multi_grep"
        ];
      };
    };
    "fff-worktree" = {
      "args" = [ "/home/user/.local/share/opencode/worktree" ];
      "command" = "fff-mcp";
      "mcpx" = {
        "readOnlyTools" = [
          "find_files"
          "grep"
          "multi_grep"
        ];
      };
    };
    "gopls-mcp-server" = {
      "args" = [
        "run"
        "golang.org/x/tools/gopls@latest"
        "mcp"
      ];
      "command" = "go";
    };
    "github" = {
      "args" = [ "stdio" ];
      "command" = "github-mcp-server";
      "auth" = {
        "type" = "env";
        "env" = {
          "GITHUB_PERSONAL_ACCESS_TOKEN" = "";
        };
        "failover" = {
          # mcpx secret names are environment-variable identifiers; map
          # these runtime slots from the corresponding Fleet source names:
          # GITHUB_APP_DEVELOPING_TODAY_FLEET_PRIVATE_KEY <- github-app.developing-today-fleet.private-key
          # KANEO_GITHUB_APP_PRIVATE_KEY <- kaneo.github-app-private-key
          # GITHUB_APP_BACKUP_PRIVATE_KEY <- github-app.backup-private-key
          # GITHUB_CLI_DEZREN39_OAUTH_TOKEN <- github-cli.hosts-yml (users.dezren39.oauth_token)
          "identities" = [
            {
              "name" = "developing-today-fleet";
              "kind" = "app";
              "secret" = "GITHUB_APP_DEVELOPING_TODAY_FLEET_PRIVATE_KEY";
              "appId" = 5612056;
              "installationId" = 178567642;
            }
            {
              "name" = "developing.today";
              "kind" = "app";
              "secret" = "KANEO_GITHUB_APP_PRIVATE_KEY";
              "appId" = 5454422;
              "installationId" = 174792676;
            }
            {
              "name" = "developing-today-backup";
              "kind" = "app";
              "secret" = "GITHUB_APP_BACKUP_PRIVATE_KEY";
              "appId" = 5562183;
              "installationId" = 177388707;
            }
            {
              "name" = "dezren39-oauth";
              "kind" = "token";
              # Derived at activation from the dezren39 oauth_token in
              # Fleet's github-cli.hosts-yml YAML secret. mcpx runtime
              # slots are environment-variable names, so these names map
              # the Fleet source secrets to raw in-memory credentials.
              "secret" = "GITHUB_CLI_DEZREN39_OAUTH_TOKEN";
            }
          ];
          "statePath" = "/home/user/.local/state/mcpx/github-failover-state.json";
          "cooldownSeconds" = 60;
        };
      };
      # A soft-block retry rotates the token at call time. Keep only one
      # caller on this process and let the pool close it so the next child
      # starts with the rotated token; a supervised child keeps its old env.
      "mcpx" = {
        "sharing" = "exclusive";
        "supervised" = false;
      };
    };
    "mcp-server-neon" = {
      "args" = [
        "-y"
        "mcp-remote"
        "https://mcp.neon.tech/sse"
      ];
      "command" = "npx";
    };
    "sequential-thinking" = {
      "args" = [
        "-y"
        "@modelcontextprotocol/server-sequential-thinking"
      ];
      "command" = "npx";
    };
  };
  "repair" = {
    "model" = "claude-code/claude-haiku-5-5";
    "provider" = "opencode";
  };
}
