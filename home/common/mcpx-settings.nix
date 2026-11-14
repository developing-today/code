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
  "mcpServers" = {
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
