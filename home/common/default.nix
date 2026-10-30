{
  lib,
  inputs,
  stateVersion,
  pkgs,
  system,
  ...
}:
let
  # Local proxy ports, each defined ONCE. Each value flows to the systemd
  # user service that binds it, to the shell session (<NAME>_PROXY_PORT),
  # and — for the proxies opencode talks to — into opencode.jsonc's
  # baseURL via {env:...} substitution, so the copies can never drift
  # apart and a new device that applies this config gets consistent ports
  # automatically. The bin/*.mjs proxies keep the same numbers as fallback
  # defaults when the env var is absent.
  clefProxyPort = 18742;
  agyProxyPort = 18743;
  codexProxyPort = 18744;
  antigravityLsProxyPort = 18745;
  designCodeProxyPort = 18746;
  museProxyPort = 18747;
  commandcodeProxyPort = 18748;
  opencodeRouteProxyPort = 18750;
  claudeCodeProxyPort = 18749;
  # Upstream prebuilt release binaries for tools where nixpkgs trails upstream.
  # See pkgs/latest-cli/default.nix for the rationale, the tradeoffs, and the
  # list of things deliberately NOT bumped.
  latestCli = pkgs.callPackage ../../pkgs/latest-cli { };
  # Vendored from nixpkgs master; see the header in that file for why. Provides
  # `agy_acp_server`, which t3code's AcpRegistryDriver can drive.
  antigravity-acp = pkgs.callPackage ../../pkgs/antigravity-acp { };
  # The hub build ships the language_server binary the LS proxy drives. Same
  # derivation as the system one, so the store path is shared, not rebuilt.
  antigravity-hub = pkgs.callPackage ../../pkgs/antigravity-hub { };
  # 2.1.293; nixpkgs is on 2.1.234. Must match the system `claude` so t3's
  # claudeAgent driver and the shell CLI are the same build.
  claude-code = pkgs.callPackage ../../pkgs/claude-code { };
  # Muse Code, Meta's coding agent. A first-class T3 Code provider: the
  # 0.1.2 driver id is literally "muse" and it drives this `muse` binary.
  muse-code = pkgs.callPackage ../../pkgs/muse-code { };
  # Pi coding agent harness. Also a T3 Code provider (driver "pi") in 0.1.2.
  pi-coding-agent = pkgs.callPackage ../../pkgs/pi-coding-agent { };
  # T3 Code desktop GUI, packaged from the upstream AppImage and
  # version-locked to the latestCli.t3 server (0.1.2): a V1 app refuses a
  # V2 server and vice versa. This shadows pkgs.t3code (nixpkgs), which the
  # home.packages runCommand block below wraps for the desktop entry.
  t3code = pkgs.callPackage ../../pkgs/t3code-desktop { };
  # Desktop Commander MCP: the local half of OpenAI's "Remote Desktop
  # Commander" plugin. See pkgs/desktop-commander for the packaging; the
  # desktop-commander-remote unit below keeps the device agent online.
  desktop-commander = pkgs.callPackage ../../pkgs/desktop-commander { };
  # Hidden-shell auth hook (1Password service account, Jules, Cloudflare).
  # Same derivation nixos/agent-auth uses, so the BASH_ENV paths agree.
  agent-shell-env = pkgs.callPackage ../../pkgs/agent-shell-env { };
  # mcpx MCP gateway and Code Mode execution runner.
  mcpx = pkgs.callPackage ../../pkgs/mcpx/package.nix { };
  # The environment the mcpx daemon runs with. It is set by the start wrapper
  # below rather than by the unit, because `mcpx upgrade` starts the successor
  # through that same wrapper, so the successor gets what a fresh start gets.
  mcpxEnvironment = {
    OPENCODE_BINARY = "${inputs.opencode.packages.${system}.opencode}/bin/opencode";
    OPENCODE_V2_BIN = "${inputs.opencode.packages.${system}.opencode}/bin/opencode";
    CLAUDE_BIN = "${claude-code}/bin/claude";
    CODEX_BIN = "${latestCli.codex}/bin/codex";
    ANTIGRAVITY_BIN = "${pkgs.antigravity-cli}/bin/agy";
    PATH = lib.makeBinPath [
      inputs.opencode.packages.${system}.opencode
      claude-code
      latestCli.codex
      pkgs.antigravity-cli
      pkgs.git
      pkgs.openssh
    ];
  };
  # The unit's start command, ahead of the binary. Its store path changes when
  # the environment or the wrapper does, and the activation keys on that.
  mcpxStart = pkgs.writeShellScript "mcpx-start" ''
    ${lib.concatStrings (
      lib.mapAttrsToList (k: v: "export ${k}=${lib.escapeShellArg v}\n") mcpxEnvironment
    )}
    exec "$@"
  '';
  mcpxKey = deployKey "mcpx" [
    mcpx
    mcpxStart
  ];
  openchamberStart = pkgs.writeShellScript "openchamber-serve" ''
    # Single-instance guard (dezren39/nix#1117). The unit below is
    # socket-activated with KillMode=process: a stop terminates only the
    # tracked main process, so a server that is no longer the tracked
    # main -- a self-handoff predecessor systemd has lost track of --
    # survives in the cgroup still holding the listen socket systemd
    # handed it. The next start passes the same socket to a new server
    # and both accept() on it, so /api/version answers alternate between
    # instances. Reap any leftover server before taking the socket over.
    # Only the web server is matched (its managed OpenCode serves a
    # different port and must stay up for adoption), and this script's
    # own pid is excluded -- it execs into the new server below.
    leftovers=""
    for proc in /proc/[0-9]*; do
      pid="''${proc#/proc/}"
      [ "$pid" = "$$" ] && continue
      # The pattern text itself rides in the argv of anything inspecting
      # the fleet (a grep, a diagnostic shell), so a cmdline match alone
      # is not proof: the process must actually be the node server.
      exe="$(readlink "$proc/exe" 2>/dev/null)" || continue
      case "$exe" in
        */node | */openchamber) ;;
        *) continue ;;
      esac
      cmd="$(cat "$proc/cmdline" 2>/dev/null | tr '\0' ' ')"
      case "$cmd" in
        *"/bin/openchamber serve"* | *"cli.js serve"*) ;;
        *) continue ;;
      esac
      case "$cmd" in
        # Argument boundary required: "--port 3000" must not match the
        # 30001..30009 of an unrelated instance (cmdline words are
        # space-separated here, trailing word included).
        *"--port 3000 "*) leftovers="$leftovers $pid" ;;
        *"--port=3000 "*) leftovers="$leftovers $pid" ;;
      esac
    done
    if [ -n "$leftovers" ]; then
      echo "openchamber-serve: reaping leftover server(s):$leftovers" >&2
      for pid in $leftovers; do
        kill -TERM "$pid" 2>/dev/null || true
      done
      # OpenChamber drains HTTP for up to 10 s on SIGTERM.
      for _ in $(seq 1 100); do
        alive=0
        for pid in $leftovers; do
          kill -0 "$pid" 2>/dev/null && alive=1
        done
        [ "$alive" = 0 ] && break
        sleep 0.1
      done
      for pid in $leftovers; do
        if kill -0 "$pid" 2>/dev/null; then
          echo "openchamber-serve: pid $pid ignored SIGTERM; killing" >&2
          kill -KILL "$pid" 2>/dev/null || true
        fi
      done
      # Verify synchronously, before taking the socket over: a leftover
      # that could not be reaped fails this start loudly instead of
      # silently sharing the listener with the new server. (An
      # ExecStartPost assertion cannot do this: with Type=simple it
      # races the guard's drain window.)
      sleep 0.2
      for pid in $leftovers; do
        if kill -0 "$pid" 2>/dev/null; then
          echo "openchamber-serve: leftover server pid $pid could not be reaped; refusing to start a second server on port 3000" >&2
          exit 1
        fi
      done
    fi
    pw="$HOME/.config/openchamber/ui-password"
    if [ -r "$pw" ]; then
      export OPENCHAMBER_UI_PASSWORD="$(< "$pw")"
    fi
    if [ -r "$HOME/.config/cloudflare/ai-inference-token" ]; then
      export CLOUDFLARE_ACCOUNT_ID="$(< "$HOME/.config/cloudflare/account-id")"
      export CLOUDFLARE_GATEWAY_ID="$(< "$HOME/.config/cloudflare/gateway-id")"
      export CLOUDFLARE_API_TOKEN="$(< "$HOME/.config/cloudflare/ai-inference-token")"
    fi
    export OPENCODE_BINARY="${openchamberBinDir}/opencode"
    export PATH="${openchamberBinDir}:$PATH"
    exec ${inputs.openchamber.packages.${system}.openchamber}/bin/openchamber serve \
      --foreground \
      --port 3000 --host 127.0.0.1
  '';
  # A unit is replaced at switch time only when its start command or binaries change.
  deployKey = name: parts: "${name}:${builtins.concatStringsSep " " (map toString parts)}";
  # OpenChamber runs the OpenCode behind this link. A switch re-points it and
  # reloads OpenChamber, so a new OpenCode takes over without a restart.
  openchamberBinDir = "$HOME/.local/state/openchamber/bin";
in
{
  wayland.windowManager.hyprland = {
    enable = true;
    #plugins = [ inputs.hypr-dynamic-cursors.packages.${pkgs.system}.hypr-dynamic-cursors ];
    # Hyprland itself is installed by the NixOS module (nixos/hyprland), which
    # pins the master flake package. Null here so home-manager does not pull a
    # second, differently-versioned hyprland from nixpkgs into $PATH.
    package = null;
    portalPackage = null;
    # Redundant at stateVersion >= 26.05, where "lua" is the default, but kept
    # explicit: the master hyprland package only reads hyprland.lua, so this is
    # load-bearing if stateVersion is ever rolled back.
    configType = "lua";
    extraConfig = builtins.readFile (lib.from-root "config/hypr/hyprland.lua");
    # settings = {
    #   "$mod" = "SUPER";
    # }
    # systemd.variables = ["--all"];
  };

  # home.pointerCursor = {
  #   gtk.enable = true;
  #   # x11.enable = true;
  #   package = pkgs.bibata-cursors;
  #   name = "Bibata-Modern-Classic";
  #   size = 16;
  # };

  # gtk = {
  #   enable = true;

  #   theme = {
  #     package = pkgs.flat-remix-gtk;
  #     name = "Flat-Remix-GTK-Grey-Darkest";
  #   };

  #   iconTheme = {
  #     package = pkgs.gnome.adwaita-icon-theme;
  #     name = "Adwaita";
  #   };

  #   font = {
  #     name = "Sans";
  #     size = 11;
  #   };
  # };
  # TODO: ensure home manager standalone can still work
  # TODO: factor out modules into shared files
  # nixpkgs.config removed: conflicts with home-manager.useGlobalPkgs (warning);
  # allowUnfree/permittedInsecure are set at the NixOS level in lib/default.nix
  gtk = {
    enable = true;
    gtk3.extraConfig.gtk-decoration-layout = "menu:";
    # cursorTheme.name = "Qogir";
    iconTheme.name = "Qogir";
    theme.name = "Jasper-Grey-Dark-Compact";
  };
  xdg = {
    enable = true;
    userDirs.enable = true;

    # CURRENT: home-manager owns ~/.config/hypr/hyprland.lua.
    # The wayland.windowManager.hyprland block above reads
    # config/hypr/hyprland.lua via `extraConfig` and wraps it with the
    # hyprland-session.target start/shutdown hooks before writing it out.
    # Everything else in config/hypr/ is a plain asset with no generation step,
    # so those are linked individually. hyprpaper.conf refers to
    # ~/.config/hypr/wallpaper.jpg, so that path must keep existing.
    configFile."hypr/hyprpaper.conf".source = lib.from-root "config/hypr/hyprpaper.conf";
    configFile."hypr/wallpaper.jpg".source = lib.from-root "config/hypr/wallpaper.jpg";

    # PREVIOUS: recursive link of the whole directory.
    #
    #   configFile."hypr" = {
    #     source = lib.from-root "config/hypr";
    #     recursive = true;
    #   };
    #
    # That linked every file in config/hypr/ verbatim -- including
    # hyprland.lua, which SHADOWED home-manager's generated config. The
    # generated file was still built but referenced zero times, so the
    # hl.on("hyprland.start", ...) hook that runs
    # `systemctl --user start hyprland-session.target` never ran. The
    # hand-rolled dbus-update-activation-environment exec at the top of
    # hyprland.lua was a partial substitute for that hook and is now redundant
    # with it (harmless, but removable).
    #
    # TO GO BACK: re-enable the block above and delete the two individual
    # configFile entries. ~/.config/hypr/hyprland.lua then becomes the raw repo
    # file again. If you do, also set
    # `wayland.windowManager.hyprland.systemd.enable = false` so the config
    # stops claiming a systemd integration that never activates.

    # lua-language-server stubs for the `hl.*` API used by hyprland.lua.
    # home-manager's hyprland module emits this itself, but only when its
    # `package` is non-null; we set that to null so the NixOS module owns the
    # install, so point it at the same flake package by hand.
    configFile."hypr/.luarc.json".text = builtins.toJSON {
      workspace.library = [
        "${inputs.hyprland.packages.${system}.hyprland}/share/hypr/stubs"
      ];
      diagnostics.globals = [ "hl" ];
    };
    mimeApps.defaultApplications = {
      "application/x-extension-htm" = "firefox.desktop";
      "application/x-extension-html" = "firefox.desktop";
      "application/x-extension-shtml" = "firefox.desktop";
      "application/x-extension-xht" = "firefox.desktop";
      "application/x-extension-xhtml" = "firefox.desktop";
      "application/xhtml+xml" = "firefox.desktop";
      "text/html" = "firefox.desktop";
      "x-scheme-handler/chrome" = "firefox.desktop";
      "x-scheme-handler/http" = "firefox.desktop";
      "x-scheme-handler/https" = "firefox.desktop";
    };
  };
  services = {
    udiskie = {
      enable = true;
    };
    mako = {
      enable = true;
      settings = {
        # anchor = "top-right";
        # anchor = "center-right";
        anchor = "bottom-right";
        border-radius = 0;
        border-size = 0;
        padding = "0"; # within
        margin = "0"; # "36,0,0,0"; # outside # 36? 40?
        # margin = "36,0,0,0"; # outside # 36? 40?
        # .tabbrowser-tab[selected] {
        #   max-height: 24px !important;
        #   min-height: 24px !important;
        # }
        # tab:not([selected="true"]) {
        #   max-height: 24px !important;
        #   min-height: 24px !important;
        # }
        # max-icon-size = 256;
        max-icon-size = 512;
        ignore-timeout = true;
        default-timeout = 15000;
        layer = "top";
        height = 240;
        width = 420;
        format = "<b>%s</b>\\n%b";
        background-color = "#303030FF";
        border-color = "#333333FF";
        # on-button-right=exec makoctl menu -n "$id" rofi -dmenu -p 'Select action: '
        # on-button-right=exec hyprctl setprop pid:$idhyprctl dispatch focuswindow
        # on-button-left=exec bash -c 'hyprctl dispatch focuswindow "pid:$1"' _ $id
        # on-button-right=exec bash -c 'hyprctl dispatch focuswindow "pid:$1"' _ $id
        # outside # 36? 40?12
        # outer-margin=36,0,0,0
        #extraConfig = ''
        #  [app-name="Element"]
        #  on-button-left=exec bash -c 'hyprctl dispatch workspace $(hyprctl -j clients | jq -r ".[] | select (.class == \"Element\") | .workspace.id")' _
        #
        #       [urgency=low]
        #      default-timeout=10000
        #
        #       [urgency=high]
        #      default-timeout=30000
        #
        #       [mode=dnd]
        #      invisible=1
        #   '';
      };
    };
    # dunst = {
    #   enable = true;
    #   package = pkgs.dunst;
    #   settings = {
    #     global = {
    #       monitor = 0;
    #       follow = "mouse";
    #       # border = 0;
    #       # height = 300;
    #       height = 360;
    #       # height = 400;
    #       # width = 320;
    #       # width = 420;
    #       # width = 480;
    #       # width = 240;
    #       # width = 320;
    #       offset = "0x0";
    #       # offset = "33x65";
    #       indicate_hidden = "yes";
    #       shrink = "yes";
    #       # shrink = "no";
    #       separator_height = 0;
    #       padding = 0;
    #       # padding = 32;
    #       # horizontal_padding = 32;
    #       horizontal_padding = 0;
    #       frame_width = 0;
    #       sort = "no";
    #       idle_threshold = 120;
    #       font = "Noto Sans";
    #       line_height = 4;
    #       markup = "full";
    #       format = "<b>%s</b>\\n%b";
    #       alignment = "left";
    #       # transparency = 10;
    #       transparency = 100;
    #       show_age_threshold = 60;
    #       word_wrap = "yes";
    #       ignore_newline = "no";
    #       stack_duplicates = false;
    #       hide_duplicate_count = "yes";
    #       show_indicators = "no";
    #       # icon_position = "off";
    #       icon_position = "left";
    #       icon_theme = "Adwaita-dark";
    #       sticky_history = "yes";
    #       history_length = 20;
    #       # browser = "google-chrome-stable";
    #       # browser = "firefox";
    #       browser = "${config.programs.firefox.package}/bin/firefox -new-tab";
    #       dmenu = "${pkgs.rofi}/bin/rofi -dmenu"; # wofi? etc.
    #       always_run_script = true;
    #       title = "Dunst";
    #       class = "Dunst";
    #       # max_icon_size = 64;
    #       max_icon_size = 128;
    #       # max_icon_size = 32;
    #       history = "ctrl+grave";
    #       context = "grave+space";
    #       close = "mod4+shift+space";
    #     };
    #   };
    # };
    activitywatch = {
      enable = true;
      package = inputs.nixpkgs-stable.legacyPackages.${pkgs.system}.activitywatch;
    };
  };
  # OpenChamber rewrites opencode's config on every launch, migrating the v1
  # `plugin` key to v2 `plugins` (and `agent` to `agents`). The system opencode
  # is 1.x and ignores `plugins` entirely, so each OpenChamber launch silently
  # stops all 13 plugins from loading for the CLI. Verified with
  # `opencode debug config` against both builds.
  #
  # Rather than fight it by hand, watch the file and normalise it back. The
  # script is idempotent and refuses to write anything it cannot re-parse.
  systemd.user.services.opencode-config-normalize = {
    Unit = {
      Description = "Normalise opencode config to v1 plugin/agent keys";
      StartLimitIntervalSec = "30s";
      StartLimitBurst = 30;
    };
    Service = {
      Type = "oneshot";
      ExecStart = "${pkgs.writeShellScript "opencode-config-normalize" (
        builtins.readFile ../../pkgs/opencode-config-normalize/normalize.sh
      )}";
      Environment = [
        "PATH=${
          lib.makeBinPath [
            pkgs.python3
            pkgs.coreutils
          ]
        }"
      ];
    };
  };
  systemd.user.paths.opencode-config-normalize = {
    Unit.Description = "Watch opencode config for OpenChamber's plugin-key rewrite";
    Path = {
      PathChanged = "%h/.config/opencode/opencode.jsonc";
      Unit = "opencode-config-normalize.service";
    };
    Install.WantedBy = [ "default.target" ];
  };

  # Dated, on-change snapshots of agent credentials and config.
  #
  # Narrow by design: the irreplaceable material is kilobytes of auth.json /
  # API keys / settings.json, while ~/.antigravity-ide and ~/.config/Antigravity
  # IDE are ~4G of regenerable cache between them. See the script header.
  #
  # Triggered by a path unit on the credential files rather than a timer, so a
  # snapshot lands when something actually changes. The script discards the new
  # directory if it is byte-identical to the previous one, so the history only
  # grows on real change.
  systemd.user.services.agent-backup = {
    Unit = {
      Description = "Snapshot agent credentials and config to a dated directory";
      StartLimitIntervalSec = "30s";
      StartLimitBurst = 30;
    };
    Service = {
      Type = "oneshot";
      ExecStart = "${pkgs.writeShellScript "agent-backup" (
        builtins.readFile ../../pkgs/agent-backup/backup.sh
      )}";
      Environment = [
        "PATH=${
          lib.makeBinPath [
            pkgs.coreutils
            pkgs.diffutils
            pkgs.findutils
          ]
        }"
      ];
    };
  };
  systemd.user.paths.agent-backup = {
    Unit.Description = "Watch agent credentials for changes";
    Path = {
      # PathChanged fires on close-after-write. Directories are watched where
      # the interesting file is rewritten rather than edited in place.
      PathChanged = [
        "%h/.config/jules/api-key"
        "%h/.local/share/opencode/auth.json"
        "%h/.config/opencode/opencode.jsonc"
        "%h/.local/share/t3code/userdata/settings.json"
        "%h/.claude.json"
        "%h/.codex/auth.json"
        "%h/.grok/auth.json"
        "%h/.pi/agent/auth.json"
        "%h/.config/cloudflare/ai-inference-token"
        "%h/.config/muse/auth.json"
        "%h/.config/openchamber/classifier-endpoint.json"
        "%h/.config/openchamber/classification.json"
      ];
      Unit = "agent-backup.service";
    };
    Install.WantedBy = [ "default.target" ];
  };

  # Seed t3's provider instances before the server starts. t3 writes to
  # settings.json at runtime so it cannot be a read-only store symlink; this
  # merges our instances in and leaves anything else (including UI-made
  # changes) untouched.
  systemd.user.services.t3code-seed-providers = {
    Unit = {
      Description = "Seed t3code provider instances";
      Before = [ "t3code.service" ];
    };
    Service = {
      Type = "oneshot";
      ExecStart = "${pkgs.writeShellScript "t3code-seed-providers" (
        builtins.readFile ../../pkgs/t3code-seed-providers/seed.sh
      )}";
      Environment = [
        "PATH=${
          lib.makeBinPath [
            pkgs.python3
            pkgs.coreutils
          ]
        }"
        "T3CODE_HOME=%h/.local/share/t3code"
        "OPENCODE_V2_BIN=${inputs.opencode.packages.${system}.opencode}/bin/opencode"
        "ANTIGRAVITY_ACP_BIN=${antigravity-acp}/bin/agy_acp_server"
        "MUSE_BIN=${muse-code}/bin/muse"
        "PI_BIN=${pi-coding-agent}/bin/pi"
      ];
    };
    Install.WantedBy = [ "default.target" ];
  };

  # OpenChamber's server, as a unit rather than a stray `openchamber` daemon.
  #
  # Running it by hand leaves a long-lived process that rewrites opencode's
  # config (plugin -> plugins) behind your back; as a unit it is at least
  # visible and restartable. The normalize path-unit above repairs the config
  # either way.
  #
  # Bound to loopback deliberately. OpenChamber warns
  # "OPENCHAMBER_UI_PASSWORD is not set / browser UI is unsecured" -- the
  # password is read from a 0600 file so it never enters the Nix store. Create
  # it with:
  #   install -m600 /dev/null ~/.config/openchamber/ui-password
  #   printf '%s' 'your-password' > ~/.config/openchamber/ui-password
  systemd.user.services.clef-proxy = {
    Unit = {
      Description = "Clef System One Local Proxy for OpenChamber";
      After = [ "network-online.target" ];
      Wants = [ "network-online.target" ];
    };
    Service = {
      ExecStart = toString (
        pkgs.writeShellScript "clef-proxy-start" ''
          exec ${pkgs.nodejs}/bin/node ${../../bin}/clef-proxy.mjs
        ''
      );
      Restart = "always";
      RestartSec = 3;
      Environment = [
        "CLEF_PROXY_PORT=${toString clefProxyPort}"
        "CLEF_PROXY_HOST=127.0.0.1"
      ];
    };
    # No Install.WantedBy: the proxy-supervisor (below) owns the proxy
    # processes. These per-proxy units remain for manual one-off starts.
  };

  # Antigravity CLI (agy) Local Proxy Bridge
  # Disabled per user request (too slow due to per-turn CLI spawning; replaced by antigravity-ls-proxy)
  systemd.user.services.agy-proxy = {
    Unit = {
      Description = "Antigravity CLI (agy) Local Proxy Bridge for OpenCode (DISABLED)";
      After = [ "network-online.target" ];
    };
    Service = {
      ExecStart = toString (
        pkgs.writeShellScript "agy-proxy-start" ''
          export PATH="$HOME/.gemini/bin:$HOME/.local/bin:$PATH"
          exec ${pkgs.nodejs}/bin/node ${../../bin}/agy-proxy.mjs
        ''
      );
      Restart = "no";
      Environment = [
        "AGY_PROXY_PORT=${toString agyProxyPort}"
        "AGY_PROXY_HOST=127.0.0.1"
      ];
    };
  };

  systemd.user.services.codex-proxy = {
    Unit = {
      Description = "OpenAI Codex CLI (codex) Local Proxy Bridge for OpenCode";
      After = [ "network-online.target" ];
      Wants = [ "network-online.target" ];
    };
    Service = {
      ExecStart = toString (
        pkgs.writeShellScript "codex-proxy-start" ''
          export PATH="/run/current-system/sw/bin:$HOME/.local/bin:$PATH"
          exec ${pkgs.nodejs}/bin/node ${../../bin}/codex-proxy.mjs
        ''
      );
      Restart = "always";
      RestartSec = 3;
      Environment = [
        "CODEX_PROXY_PORT=${toString codexProxyPort}"
        "CODEX_PROXY_HOST=127.0.0.1"
      ];
    };
    # No Install.WantedBy: owned by the proxy-supervisor (below).
  };

  systemd.user.services.antigravity-ls-proxy = {
    Unit = {
      Description = "Antigravity Language Server Direct gRPC Proxy Bridge for OpenCode";
      After = [ "network-online.target" ];
      Wants = [ "network-online.target" ];
    };
    Service = {
      ExecStart = toString (
        pkgs.writeShellScript "antigravity-ls-proxy-start" ''
          export PATH="/run/current-system/sw/bin:$HOME/.gemini/bin:$HOME/.local/bin:$PATH"
          exec ${pkgs.nodejs}/bin/node ${../../bin}/antigravity-ls-proxy.mjs
        ''
      );
      Restart = "always";
      RestartSec = 3;
      Environment = [
        "ANTIGRAVITY_LS_PROXY_PORT=${toString antigravityLsProxyPort}"
        "ANTIGRAVITY_LS_PROXY_HOST=127.0.0.1"
        "ANTIGRAVITY_LS_BIN=${antigravity-hub}/share/antigravity/resources/bin/language_server"
      ];
    };
    # No Install.WantedBy: owned by the proxy-supervisor (below).
  };

  systemd.user.services.design-code-proxy = {
    Unit = {
      Description = "Code Design / Design Code Local Proxy Bridge for OpenCode";
      After = [
        "network-online.target"
        "antigravity-ls-proxy.service"
        "codex-proxy.service"
      ];
      Wants = [
        "network-online.target"
        "antigravity-ls-proxy.service"
        "codex-proxy.service"
      ];
    };
    Service = {
      ExecStart = toString (
        pkgs.writeShellScript "design-code-proxy-start" ''
          export PATH="/run/current-system/sw/bin:$HOME/.local/bin:$PATH"
          exec ${pkgs.nodejs}/bin/node ${../../bin}/design-code-proxy.mjs
        ''
      );
      Restart = "always";
      RestartSec = 3;
      Environment = [
        "DESIGN_CODE_PROXY_PORT=${toString designCodeProxyPort}"
        "DESIGN_CODE_PROXY_HOST=127.0.0.1"
      ];
    };
    # No Install.WantedBy: owned by the proxy-supervisor (below).
  };

  # Fleet proxy supervisor (bin/proxy-supervise.mjs): hash-keyed A/B
  # rolls for the proxies above. Drew's rule (2026-10-24): a rebuild +
  # switch must not kill a running proxy unless the artifact that
  # would run actually changed — and then the roll is graceful (new
  # build on the alternate port, health-checked, endpoint record
  # flips, old instance drained). The supervisor therefore owns the
  # proxy processes; this unit only runs the reconcile, and it invokes
  # the supervisor from the CHECKOUT path so this unit's definition
  # does not change when proxy code does.
  systemd.user.services.proxy-supervisor = {
    Unit = {
      Description = "Fleet proxy supervisor (hash-keyed A/B reconcile)";
      After = [ "network-online.target" ];
      Wants = [ "network-online.target" ];
    };
    Service = {
      Type = "oneshot";
      ExecStart = toString (
        pkgs.writeShellScript "proxy-supervisor-run" ''
          exec /run/current-system/sw/bin/node "$HOME/code/bin/proxy-supervise.mjs" --config "$HOME/.config/proxy-supervisor/config.json"
        ''
      );
      Environment = [
        # Interpolated into the supervisor config ($ANTIGRAVITY_LS_BIN);
        # it is part of the LS proxy's artifact hash, so a hub rebuild
        # rolls that proxy gracefully instead of stranding it.
        "ANTIGRAVITY_LS_BIN=${antigravity-hub}/share/antigravity/resources/bin/language_server"
      ];
    };
  };

  systemd.user.timers.proxy-supervisor = {
    Unit = {
      Description = "Reconcile fleet proxies every 2 minutes";
    };
    Timer = {
      OnBootSec = "1min";
      OnUnitActiveSec = "2min";
    };
    Install.WantedBy = [ "timers.target" ];
  };

  # The supervisor's machine config. "~" and "$VAR" in paths/values are
  # expanded by the supervisor itself.
  xdg.configFile."proxy-supervisor/config.json".text = builtins.toJSON {
    stateDir = "~/.local/state/proxy-supervisor";
    endpointDir = "~/.local/state";
    systemdRun = "user";
    drainSeconds = 5;
    healthTimeoutSeconds = 60;
    proxies = [
      {
        name = "clef-proxy";
        canonicalPort = clefProxyPort;
        portEnv = "CLEF_PROXY_PORT";
        command = [
          "/run/current-system/sw/bin/node"
          "~/code/bin/clef-proxy.mjs"
        ];
        env = {
          CLEF_PROXY_HOST = "127.0.0.1";
        };
        hashFiles = [
          "~/code/bin/clef-proxy.mjs"
          "~/code/bin/proxy-port.mjs"
        ];
      }
      {
        name = "agy-proxy";
        canonicalPort = agyProxyPort;
        portEnv = "AGY_PROXY_PORT";
        command = [
          "/run/current-system/sw/bin/node"
          "~/code/bin/agy-proxy.mjs"
        ];
        env = {
          AGY_PROXY_HOST = "127.0.0.1";
          PATH = "/run/current-system/sw/bin:$HOME/.gemini/bin:$HOME/.local/bin:/usr/bin:/bin";
        };
        hashFiles = [
          "~/code/bin/agy-proxy.mjs"
          "~/code/bin/proxy-port.mjs"
        ];
      }
      {
        name = "codex-proxy";
        canonicalPort = codexProxyPort;
        portEnv = "CODEX_PROXY_PORT";
        command = [
          "/run/current-system/sw/bin/node"
          "~/code/bin/codex-proxy.mjs"
        ];
        env = {
          CODEX_PROXY_HOST = "127.0.0.1";
          PATH = "/run/current-system/sw/bin:$HOME/.local/bin:/usr/bin:/bin";
        };
        hashFiles = [
          "~/code/bin/codex-proxy.mjs"
          "~/code/bin/proxy-port.mjs"
          # Build stamp: a content change here stands in for "the
          # artifact changed" (used by the A/B demo; harmless in
          # production — a missing file hashes as a constant).
          "~/.local/state/proxy-supervisor/codex-proxy.build-stamp"
        ];
      }
      {
        name = "antigravity-ls-proxy";
        canonicalPort = antigravityLsProxyPort;
        portEnv = "ANTIGRAVITY_LS_PROXY_PORT";
        command = [
          "/run/current-system/sw/bin/node"
          "~/code/bin/antigravity-ls-proxy.mjs"
        ];
        env = {
          ANTIGRAVITY_LS_PROXY_HOST = "127.0.0.1";
          ANTIGRAVITY_LS_BIN = "$ANTIGRAVITY_LS_BIN";
          PATH = "/run/current-system/sw/bin:$HOME/.gemini/bin:$HOME/.local/bin:/usr/bin:/bin";
        };
        hashEnv = [ "ANTIGRAVITY_LS_BIN" ];
        hashFiles = [
          "~/code/bin/antigravity-ls-proxy.mjs"
          "~/code/bin/proxy-port.mjs"
        ];
      }
      {
        name = "design-code-proxy";
        canonicalPort = designCodeProxyPort;
        portEnv = "DESIGN_CODE_PROXY_PORT";
        command = [
          "/run/current-system/sw/bin/node"
          "~/code/bin/design-code-proxy.mjs"
        ];
        env = {
          DESIGN_CODE_PROXY_HOST = "127.0.0.1";
          PATH = "/run/current-system/sw/bin:$HOME/.local/bin:/usr/bin:/bin";
        };
        hashFiles = [
          "~/code/bin/design-code-proxy.mjs"
          "~/code/bin/proxy-port.mjs"
        ];
        backends = [
          {
            name = "codex-proxy";
            env = "CODEX_PROXY_PORT";
          }
          {
            name = "antigravity-ls-proxy";
            env = "ANTIGRAVITY_LS_PROXY_PORT";
          }
        ];
      }
      # The two entries below were added to the LIVE supervisor config by
      # hand on 2026-10-24 but never made it back into this file -- the drift
      # surfaced on 2026-10-26, when home-manager activation failed on the
      # hand-placed (unmanaged) supervisor unit/config files and the live
      # config turned out to supervise two more proxies than the repo knew
      # about. Transcribed verbatim from the live config: unlike the node
      # proxies above these run their repo start scripts (which own their
      # environment, cf. the commandcode-proxy unit below), so they carry
      # no `env` block.
      {
        name = "commandcode-proxy";
        canonicalPort = commandcodeProxyPort;
        portEnv = "COMMANDCODE_PROXY_PORT";
        command = [
          "/run/current-system/sw/bin/bash"
          "~/code/bin/commandcode-proxy-start.sh"
        ];
        hashFiles = [
          "~/code/bin/commandcode-proxy.mjs"
          "~/code/bin/proxy-port.mjs"
          "~/.local/state/proxy-supervisor/commandcode-proxy.build-stamp"
        ];
      }
      {
        name = "muse-proxy";
        canonicalPort = museProxyPort;
        portEnv = "MUSE_PROXY_PORT";
        command = [
          "/run/current-system/sw/bin/bash"
          "~/code/bin/muse-proxy-start.sh"
        ];
        hashFiles = [
          "~/code/bin/muse-proxy.mjs"
          "~/code/bin/proxy-port.mjs"
          "~/.local/state/proxy-supervisor/muse-proxy.build-stamp"
        ];
      }
      {
        name = "opencode-route-proxy";
        # swap, not the supervisor default "ab": OpenClaw's provider
        # base URL is the literal canonical port (18750) and nothing
        # consumes the endpoint record to follow an ab roll to 19750,
        # so the door would strand. swap validates the new build on
        # the alternate port, then restarts it on the canonical port.
        strategy = "swap";
        canonicalPort = opencodeRouteProxyPort;
        portEnv = "OPENCODE_ROUTE_PROXY_PORT";
        command = [
          "/run/current-system/sw/bin/bash"
          "~/code/bin/opencode-route-proxy-start.sh"
        ];
        hashFiles = [
          "~/code/bin/opencode-route-proxy.mjs"
          "~/code/bin/proxy-port.mjs"
          "~/.local/state/proxy-supervisor/opencode-route-proxy.build-stamp"
        ];
      }
      # Enrolled 2026-10-27. The let block reserved claudeCodeProxyPort
      # and the session variables already export CLAUDE_CODE_PROXY_PORT
      # (opencode.jsonc dials it), but the proxy was never added to this
      # list, so nothing supervised port 18749 and it sat dark until
      # started by hand -- the same drift class as the hand-added pair
      # transcribed above. Entry mirrors codex-proxy (the other node
      # CLI bridge): host + PATH env, entrypoint + proxy-port.mjs as
      # the artifact hash.
      {
        name = "claude-code-proxy";
        canonicalPort = claudeCodeProxyPort;
        portEnv = "CLAUDE_CODE_PROXY_PORT";
        command = [
          "/run/current-system/sw/bin/node"
          "~/code/bin/claude-code-proxy.mjs"
        ];
        env = {
          CLAUDE_CODE_PROXY_HOST = "127.0.0.1";
          PATH = "/run/current-system/sw/bin:$HOME/.local/bin:/usr/bin:/bin";
        };
        hashFiles = [
          "~/code/bin/claude-code-proxy.mjs"
          "~/code/bin/proxy-port.mjs"
        ];
      }
    ];
  };

  # Command Code CLI Local Proxy Bridge for OpenCode (Go plan: the hosted
  # Provider API rejects the Go plan, so OpenCode talks to the CLI via
  # this OpenAI-compatible bridge on 18748, same pattern as codex-proxy).
  # Runs the repo's start script (shared with the fleet proxy-supervisor's
  # transient fps-commandcode-proxy unit) so laptop/sandbox env handling
  # and the MAX_TURNS pins live in exactly one place:
  # bin/commandcode-proxy-start.sh.
  #
  # This unit originally landed (169d594e) WITH an Install.WantedBy and a
  # "not yet deployed" note, breaking the fleet pattern below: the first
  # home-manager activation that included it (2026-10-26, the T3 switch)
  # started it against the supervisor's already-running transient unit,
  # and it crash-looped on the taken port (44 restarts in an evening).
  # The supervisor has served this port continuously since 2026-10-24;
  # the unit is the manual fallback, like every other proxy here.
  systemd.user.services.commandcode-proxy = {
    Unit = {
      Description = "Command Code CLI Local Proxy Bridge for OpenCode";
      After = [ "network-online.target" ];
      Wants = [ "network-online.target" ];
    };
    Service = {
      ExecStart = "${../../bin}/commandcode-proxy-start.sh";
      Restart = "always";
      RestartSec = 3;
      Environment = [
        "COMMANDCODE_PROXY_PORT=${toString commandcodeProxyPort}"
        "COMMANDCODE_PROXY_HOST=127.0.0.1"
        "COMMANDCODE_PROXY_MAX_TURNS=1"
        "COMMANDCODE_PROXY_AGENCY_MAX_TURNS=8"
      ];
    };
    # No Install.WantedBy: the proxy-supervisor (above) owns the proxy
    # processes. These per-proxy units remain for manual one-off starts.
  };

  # OpenAI-compatible front for the OpenCode server (:13420): exposes the
  # opencode-route pool models (workhorse et al.) in the chat-completions
  # shape OpenClaw providers use (code#2229). Runs the repo start script,
  # shared with the proxy-supervisor's transient unit. Manual fallback
  # only, like every other proxy unit here.
  systemd.user.services.opencode-route-proxy = {
    Unit = {
      Description = "OpenCode route proxy (OpenAI-compatible front for the OpenCode server)";
      After = [ "network-online.target" ];
      Wants = [ "network-online.target" ];
    };
    Service = {
      ExecStart = "${../../bin}/opencode-route-proxy-start.sh";
      Restart = "always";
      RestartSec = 3;
      Environment = [
        "OPENCODE_ROUTE_PROXY_PORT=${toString opencodeRouteProxyPort}"
      ];
    };
    # No Install.WantedBy: the proxy-supervisor (above) owns the proxy
    # processes. These per-proxy units remain for manual one-off starts.
  };

  systemd.user.services.sync-models = {
    Unit = {
      Description = "Sync AI models for OpenCode providers from models.dev, agy, and Codex";
      After = [ "network-online.target" ];
    };
    Service = {
      Type = "oneshot";
      ExecStart = toString (
        pkgs.writeShellScript "sync-models-run" ''
          export PATH="/run/current-system/sw/bin:$HOME/.gemini/bin:$HOME/.local/bin:$PATH"
          exec ${pkgs.nodejs}/bin/node ${../../bin/sync-models.mjs}
        ''
      );
    };
  };

  systemd.user.timers.sync-models = {
    Unit = {
      Description = "Periodically sync AI models for OpenCode providers";
    };
    Timer = {
      OnCalendar = "daily";
      Persistent = true;
      RandomizedDelaySec = "1h";
    };
    Install.WantedBy = [ "timers.target" ];
  };

  # Desktop Commander's Remote Device: the local half of OpenAI's "Remote
  # Desktop Commander" plugin. The hosted Remote MCP at
  # mcp.desktopcommander.app forwards tool calls to this process, which drives
  # the local Desktop Commander MCP server under the user's own permissions.
  # Installing the plugin itself is a ChatGPT/Codex action and cannot be done
  # by a NixOS unit; what this unit buys is that the device is online whenever
  # the user session is, rather than living in a stray terminal.
  #
  # FIRST RUN IS INTERACTIVE. `remote` starts an OAuth device flow and prints a
  # verification URL and code, so the unit only pairs unattended once a session
  # has been stored. Watch it with:
  #   journalctl --user -u desktop-commander-remote -f
  # and approve at mcp.desktopcommander.app. The session is saved to
  # ~/.desktop-commander-device/device.json (0600) and reused on restart.
  #
  # PATH matters for the same reason it does in t3code below: a systemd user
  # unit does not inherit the login shell's PATH, and Desktop Commander exists
  # to run arbitrary user commands, so it needs the real one. Sourcing the
  # profiles is deliberately broader than a makeBinPath list -- a curated set
  # would silently hide most of what the remote shell is meant to reach. The
  # agent's own ripgrep is already on its wrapped PATH from the derivation.
  systemd.user.services.desktop-commander-remote = {
    Unit = {
      Description = "Desktop Commander Remote MCP device agent";
      Documentation = "https://mcp.desktopcommander.app";
      After = [ "network-online.target" ];
      Wants = [ "network-online.target" ];
    };
    Service = {
      ExecStart = toString (
        pkgs.writeShellScript "desktop-commander-remote-start" ''
          export PATH="/run/current-system/sw/bin:/run/wrappers/bin:/etc/profiles/per-user/$USER/bin:$HOME/.nix-profile/bin:$PATH"
          exec ${desktop-commander}/bin/desktop-commander remote
        ''
      );
      # The device reconnects after a network blip on its own, so only a crash
      # or a revoked authorization should take the unit down: on-failure, with
      # a slow restart so a failing OAuth flow cannot spin.
      Restart = "on-failure";
      RestartSec = 15;
    };
    Install.WantedBy = [ "default.target" ];
  };

  # mcpx standing daemon: pool management and Code Mode runner.
  # Runs independently on boot; survives openchamber/opencode restarts.
  # Supports seamless hot reload via `mcpx reload` on SIGHUP or config changes.
  systemd.user.services.mcpx = {
    Unit = {
      Description = "mcpx standing MCP gateway daemon";
      After = [ "network-online.target" ];
      Wants = [ "network-online.target" ];
      # A switch must not restart the daemon: that would end every child.
      # mcpxDeploy below hands a changed daemon over instead.
      "X-RestartIfChanged" = false;
    };
    Service = {
      Type = "notify";
      NotifyAccess = "all";
      ExecStart = "${mcpxStart} ${mcpx}/bin/mcpx daemon";
      ExecReload = "${mcpx}/bin/mcpx reload";
      Restart = "always";
      RestartSec = 3;
    };
    Install.WantedBy = [ "default.target" ];
  };

  # A switch that changes mcpx, its start command or its environment hands the
  # running daemon over, with the MCP children and the listener passed along.
  # The successor is started through the unit's start command, so it runs with
  # the new environment. Unchanged mcpx is left alone. Falls back to a plain
  # start when no daemon is running.
  home.activation.mcpxDeploy = lib.hm.dag.entryAfter [ "reloadSystemd" ] ''
    state="''${XDG_STATE_HOME:-$HOME/.local/state}/mcpx/deployed"
    key='${mcpxKey}'
    if [ "$(cat "$state" 2>/dev/null)" != "$key" ]; then
      if ${pkgs.systemd}/bin/systemctl --user is-system-running --quiet 2>/dev/null; then
        if ${pkgs.systemd}/bin/systemctl --user is-active --quiet mcpx.service; then
          run ${mcpx}/bin/mcpx upgrade || run ${pkgs.systemd}/bin/systemctl --user restart mcpx.service
        else
          run ${pkgs.systemd}/bin/systemctl --user start mcpx.service || true
        fi
      fi
      run mkdir -p "$(dirname "$state")"
      run sh -c 'printf "%s\n" "$1" > "$2"' _ "$key" "$state"
    fi
  '';

  # Watch .mcpx.json to hot-reload the standing mcpx daemon seamlessly.
  systemd.user.paths.mcpx-config = {
    Unit.Description = "Watch .mcpx.json for live reload";
    Path = {
      PathChanged = [
        "%h/code/.mcpx.json"
        "%h/.config/mcpx/config.json"
      ];
      Unit = "mcpx-reload.service";
    };
    Install.WantedBy = [ "default.target" ];
  };

  systemd.user.services.mcpx-reload = {
    Unit.Description = "Reload mcpx daemon configuration";
    Service = {
      Type = "oneshot";
      ExecStart = "${mcpx}/bin/mcpx reload";
    };
  };

  # systemd holds the listener across restarts, so connections made during a
  # restart queue for the new process instead of being refused.
  systemd.user.sockets.openchamber = {
    Unit.Description = "OpenChamber listener";
    Socket.ListenStream = "127.0.0.1:3000";
    Install.WantedBy = [ "sockets.target" ];
  };

  # The provider proxies are owned by the proxy-supervisor (transient
  # fps-* units on the canonical ports), so OpenChamber must not Want
  # the manual per-proxy units: a restart would re-pull them, and the
  # manual design-code-proxy unit fights the supervisor's instance for
  # its already-taken port (#883). OpenChamber reaches the proxies by
  # port, not by unit; only mcpx is a real unit dependency. The manual
  # units stay declared in this file as one-off fallbacks, and the
  # After ordering entries stay (ordering only, harmless).
  systemd.user.services.openchamber = {
    Unit = {
      Description = "OpenChamber server";
      After = [
        "network-online.target"
        "mcpx.service"
        "clef-proxy.service"
        "agy-proxy.service"
        "codex-proxy.service"
        "antigravity-ls-proxy.service"
        "design-code-proxy.service"
        "openchamber.socket"
      ];
      Wants = [
        "network-online.target"
        "mcpx.service"
      ];
      Requires = [ "openchamber.socket" ];
    };
    Service = {
      ExecStart = "${openchamberStart}";
      ExecReload = "${pkgs.coreutils}/bin/kill -HUP $MAINPID";
      # Stopping OpenChamber leaves its managed OpenCode running; the next
      # OpenChamber adopts it, so sessions survive a restart. The same
      # KillMode lets an untracked server survive a stop with the socket
      # attached -- the start script reaps leftovers (dezren39/nix#1117).
      KillMode = "process";
      # A self-handoff successor names itself the main pid (MAINPID=) from a child process.
      # With NotifyAccess=none systemd ignores it, so the unit stops when the old process exits.
      NotifyAccess = "all";
      Restart = "on-failure";
      RestartSec = 5;
      Environment = [
        "PATH=${
          lib.makeBinPath [
            mcpx
            pkgs.git
            pkgs.openssh
            # The start script's single-instance guard scans /proc with
            # coreutils (readlink, cat, tr, seq, sleep); without it the
            # guard silently matches nothing.
            pkgs.coreutils
          ]
        }"
      ];
    };
    Install.WantedBy = [ "default.target" ];
  };

  # A new OpenChamber start command restarts the unit. A new OpenCode only moves
  # the link and reloads OpenChamber, which hands its OpenCode over in place.
  home.activation.openchamberDeploy = lib.hm.dag.entryAfter [ "reloadSystemd" ] ''
    state="''${XDG_STATE_HOME:-$HOME/.local/state}/openchamber/deployed"
    key='${deployKey "openchamber" [ openchamberStart ]}'
    link="$HOME/.local/state/openchamber/bin/opencode"
    target="${inputs.opencode.packages.${system}.opencode}/bin/opencode"
    previous="$(readlink "$link" 2>/dev/null || true)"
    active=0
    if ${pkgs.systemd}/bin/systemctl --user is-system-running --quiet 2>/dev/null; then
      if ${pkgs.systemd}/bin/systemctl --user is-active --quiet openchamber.service 2>/dev/null; then
        active=1
      fi
    fi
    run mkdir -p "$HOME/.local/state/openchamber/bin"
    run ln -sfn "$target" "$link"
    if [ "$(cat "$state" 2>/dev/null)" != "$key" ]; then
      if [ "$active" = 1 ]; then
        run ${pkgs.systemd}/bin/systemctl --user restart openchamber.service
      fi
      run mkdir -p "$(dirname "$state")"
      run sh -c 'printf "%s\n" "$1" > "$2"' _ "$key" "$state"
    elif [ "$active" = 1 ] && [ "$previous" != "$target" ]; then
      run ${pkgs.systemd}/bin/systemctl --user reload openchamber.service
    fi
  '';

  # T3 Code's backend, run headless over the tailnet.
  #
  # The server package (latestCli.t3) ships a `t3` binary whose whole purpose
  # is "Run the T3 Code server":
  #   t3 serve   -- server, no browser, prints headless pairing details
  #   t3 pair    -- mints a pairing token and renders it as a QR code
  #   t3 auth    -- auth control plane for headless deployments
  #   t3 thread / t3 project -- drive this server from other agents' shells
  #
  # 0.1.2 is Orchestrator V2: delegate_task spawns child threads on any
  # provider, the T3 Code MCP lets agents create/message/wait on threads, and
  # Muse Code is a native provider (driver "muse", below). T3 is a second
  # control surface over the same provider CLIs OpenChamber drives -- it owns
  # no provider credentials of its own. See
  # doc/2026-10-26T21-18-21Z_feature_t3code-v2-integration/ for the
  # coexistence line with OpenChamber.
  #
  # --tailscale-serve is a first-class upstream flag ("Configure Tailscale Serve
  # to expose this backend over HTTPS on the Tailnet"), so no tunnel, no
  # reverse proxy and no public exposure: the listener stays on loopback and
  # Tailscale terminates HTTPS on the tailnet only. NOTE: `tailscale` MUST be
  # on this unit's PATH (below) or --tailscale-serve fails -- silently from
  # the user's side, with only a TailscaleCommandExitError in the journal and
  # no HTTPS entry in `tailscale serve status`. That was the live state of
  # the 0.0.45 install.
  #
  # Defined here rather than via `t3 service install`, which would create the
  # same unit imperatively as untracked state outside the flake.
  systemd.user.services.t3code = {
    Unit = {
      Description = "T3 Code headless server (Tailscale Serve)";
      Documentation = "https://t3.codes";
      # Tailscale runs as a system service; this only needs the network up.
      After = [ "network-online.target" ];
      Wants = [ "network-online.target" ];
    };
    Service = {
      # `serve` rather than `start`: start opens a browser, which is meaningless
      # for a background unit.
      ExecStart = "${latestCli.t3}/bin/t3 serve --tailscale-serve --no-browser";
      Restart = "on-failure";
      RestartSec = 5;
      # t3 keeps runtime state under T3CODE_HOME (equivalently --base-dir).
      #
      # PATH matters here. t3 is an orchestrator: it drives other coding agents
      # through provider drivers (opencode, codex, claudeAgent, muse, pi,
      # antigravity, cursor, grok) and discovers them on PATH. A systemd user
      # unit does NOT inherit the login shell's PATH, so without this it would
      # find none of them and every provider would show as unavailable.
      #
      # Provider minimums for t3 0.1.2 are noted next to each entry; the
      # fleet builds all satisfy them.
      #
      # t3 drives the same opencode build as the system `opencode`; its binary
      # is the one noted in OPENCODE_V2_BIN below. Only OpenCode 2.x binaries
      # may touch the shared session DB -- never a 1.x build.
      #
      # `muse` is a first-class t3 driver in 0.1.2 (driver id "muse"): t3
      # checks for the `muse` CLI and its channel manifest, drives sessions
      # through it, and can inject session MCP servers into muse sessions.
      # Muse is signed in on this host under its own login; t3 owns no Muse
      # credentials.
      Environment = [
        "T3CODE_HOME=%h/.local/share/t3code"
        # No upstream telemetry from a fleet service.
        "T3CODE_TELEMETRY_ENABLED=false"
        "PATH=${
          lib.makeBinPath [
            inputs.opencode.packages.${system}.opencode # 2.0.30, >= 2.0.18 -- driver "opencode"
            latestCli.codex # 0.160.0, >= 0.159   -- driver "codex"
            claude-code # 2.1.293, >= 2.1.280 -- driver "claudeAgent"
            muse-code # 1.4.13            -- driver "muse"
            pi-coding-agent # 0.84.3, >= 0.80.5   -- driver "pi"
            mcpx # fleet MCP gateway binary, for shells and agents t3 spawns
            latestCli.gh # `gh` for threads that shell out to GitHub
            pkgs.antigravity-cli # binary is `agy` -- t3 driver "antigravity"
            pkgs.tailscale # MUST be here or --tailscale-serve fails (see above)
            pkgs.git
            pkgs.openssh
          ]
        }"
        # Explicit binary paths for provider instances that cannot be
        # auto-discovered, because t3 keeps providerInstances in its server
        # settings rather than a config file. Read these off the running unit
        # and paste them into the t3 UI as each instance's binaryPath:
        #
        #   systemctl --user show t3code -p Environment | tr ' ' '\n' | grep _BIN=
        #
        "OPENCODE_V2_BIN=${inputs.opencode.packages.${system}.opencode}/bin/opencode"
        "CODEX_BIN=${latestCli.codex}/bin/codex"
        "CLAUDE_BIN=${claude-code}/bin/claude"
        "MUSE_BIN=${muse-code}/bin/muse"
        "PI_BIN=${pi-coding-agent}/bin/pi"
        "ANTIGRAVITY_BIN=${pkgs.antigravity-cli}/bin/agy"
        # The ACP route: t3 ships an AcpRegistryDriver, and this is Google's
        # official Agent Client Protocol server for Antigravity. Preferred over
        # the bare `agy` driver where ACP is supported, since it is the generic
        # protocol path rather than a vendor-specific shim.
        "ANTIGRAVITY_ACP_BIN=${antigravity-acp}/bin/agy_acp_server"
      ];
    };
    Install.WantedBy = [ "default.target" ];
  };
  # environment.d: every systemd user unit (t3code, mcpx, desktop-commander, ...)
  # and dbus-activated app gets BASH_ENV, so the `bash -c` shells they spawn
  # load the auth hook. Backup to the pam_env copy set in nixos/agent-auth.
  systemd.user.sessionVariables.BASH_ENV = "${agent-shell-env}/share/agent-shell-env/env.sh";
  manual.manpages.enable = true;
  programs = {
    # Secrets that must NOT end up in the Nix store. home.sessionVariables and
    # systemd Environment= both bake their values into world-readable store
    # paths, so API keys cannot live there. Instead sops-nix decrypts them to
    # /run/secrets/agent-auth/* (nixos/agent-auth, owner user, 0400) and the
    # agent-shell-env hook reads them at shell startup; only PATHS are in the
    # store. Legacy 0600 files under ~/.config are still read as a fallback:
    #
    #   bws-access-token          | ~/.config/bws/access-token          -> BWS_ACCESS_TOKEN (bws login)
    #   op-service-account-token  | ~/.config/op/service-account-token  -> OP_SERVICE_ACCOUNT_TOKEN
    #   jules-api-key             | ~/.config/jules/api-key             -> JULES_API_KEY
    #   cloudflare-{account-id,gateway-id,ai-inference-token} | ~/.config/cloudflare/...
    #
    # The hook is sourced from bashrcExtra, which home-manager places BEFORE the
    # `[[ $- == *i* ]] || return` guard (initExtra comes after it, which is why
    # these exports used to be invisible to hidden agent shells). Non-interactive
    # `bash -c` shells get it through BASH_ENV instead (nixos/agent-auth +
    # systemd.user.sessionVariables below). See pkgs/agent-shell-env/env.sh.
    #
    # JULES_API_KEY is consumed by the @google/jules-mcp server wired into
    # opencode via "mcp.jules" -> environment -> {env:JULES_API_KEY}. The `jules`
    # CLI ignores it and uses the `jules login` OAuth token in gnome-keyring.
    bash = {
      bashrcExtra = ''
        . "${agent-shell-env}/share/agent-shell-env/env.sh"
      '';
    };
    # zen-browser = {
    #   enable = true;
    #   # package = inputs.zen-browser.packages.${system}.default;
    #   policies = {
    #     BlockAboutConfig = true;
    #   };
    # };
    # atuin = {
    #   enable = true;
    #   settings = {
    #     auto_sync = true;
    #     sync_frequency = "1m";
    #     sync_address = "https://api.atuin.sh";
    #     search_mode = "prefix";
    #   };
    #   flags = [
    #     "--disable-up-arrow"
    #     # "--disable-ctrl-r"
    #   ];
    # };
    ghostty = {
      enable = true;
      package = inputs.nixpkgs-master.legacyPackages.${system}.ghostty;
      settings = {
        # ghostty +list-themes
        theme = "Synthwave";
        window-decoration = false;
        # TODO: hide tabs or make smaller or both
      };
    };
    bash.enable = true;
    waybar = import (lib.from-root "home/common/programs/waybar.nix") { inherit pkgs; };
    #alacritty = import (lib.from-root "home/common/programs/alacritty.nix");
    #kitty = import (lib.from-root "home/common/programs/kitty.nix");
    yazi = import (lib.from-root "home/common/programs/yazi.nix") { inherit pkgs; };
    abook.enable = true;
    autojump.enable = true;

    autorandr.enable = true;
    # bash.enable = true; # bashrc overrides my bashrc hmmm
    bashmount.enable = true;
    # chromium.enable = true; # long build times
    dircolors.enable = true;
    direnv = {
      enable = true;
      enableZshIntegration = true;
      config = {
        whitelist = {
          prefix = [
            "~/.local/share/opencode/worktree"
            "/home/user/.local/share/opencode/worktree"
            "/root/.local/share/opencode/worktree"
            "~/code"
            "/home/user/code"
            "/root/code"
          ];
        };
      };
    };
    emacs.enable = true;
    # eww.enable = true; # config
    #eza.enable = true;
    firefox = {
      enable = true;
      # pin legacy path explicitly; default changed to xdg.configHome in 26.05
      configPath = ".mozilla/firefox";
      policies = {
        BlockAboutConfig = true;
      };
    };
    fuzzel = {
      enable = true;
      settings = {
        main = {
          #font = "Sarasa Mono SC";
          terminal = "foot";
          prompt = "->";
        };

        border = {
          width = 0;
          radius = 6;
        };

        dmenu = {
          mode = "text";
        };
        # colors = {
        #   background = "${config.color.base00}f2";
        #   text = "${config.color.base05}ff";
        #   match = "${config.color.base0A}ff";
        #   selection = "${config.color.base03}ff";
        #   selection-text = "${config.color.base05}ff";
        #   selection-match = "${config.color.base0A}ff";
        #   border = "${config.color.base0D}ff";
        # };
      };
    };
    fzf = {
      enable = true;
      # mcfly owns Ctrl-R for bash; avoid double-binding
      enableBashIntegration = false;
    };
    gh = {
      enable = true;
      # nixpkgs trails upstream (2.97.0 vs 2.102.0); prebuilt release binary.
      package = latestCli.gh;
      extensions = [ pkgs.gh-dash ]; # gh dash: TUI for PRs/issues
    };
    # git-credential-oauth.enable = true; # can't get browser to return back
    git = {
      # TODO: global config
      enable = true;
      lfs.enable = true;
      settings = {
        user = {
          name = "Drewry Pope";
          email = "drewrypope@gmail.com";
        };
        aliases = {
          ci = "commit";
          co = "checkout";
          s = "status";
        };

        # extraConfig = {
        push = {
          autoSetupRemote = true;
        };
        pull = {
          # rebase = true;
          rebase = false;
          # ff = "only";
        };
        safe = {
          directory = "*";
        };
        help.autocorrect = "immediate";
        init.defaultBranch = "main";
        #   credential.helper = "${
        #       pkgs.git.override { withLibsecret = true; }
        #     }/bin/git-credential-libsecret";
        # };
      };
      # signing.signByDefault = true;
      # gitCliff
      # difftastic
      # diff-so-fancy
      # diff-highlight
      # delta
      # gitui
      #
      # attributes = [
      #   "*.pdf diff=pdf"
      # ];

      maintenance = {
        repositories = [ "/home/user/code" ];
        timers = {
          daily = "Tue..Sun *-*-* 0:53:00";
          hourly = "*-*-* 1..23:53:00";
          weekly = "Mon 0:53:00";
        };
      };
    };
    gitui.enable = true;
    # gnome-terminal.enable = true; # strange error, probably because i'm not using gnome. interesting.
    go.enable = true;
    gpg.enable = true;
    havoc.enable = true;
    #     helix.enable = true; # try again vs binary? didn't like editor override.
    # hexchat.enable = true; # removed from nixpkgs: HexChat archived upstream, GTK2
    # htop.enable = true;
    i3status-rust.enable = true;
    i3status.enable = true;
    info.enable = true;
    irssi.enable = true;
    java.enable = true;
    jq.enable = true;
    jujutsu.enable = true;
    # just.enable = true;
    kakoune.enable = true;
    #kitty.enable = true;
    lazygit.enable = true;
    ledger.enable = true;
    less.enable = true;
    lesspipe.enable = true;
    lf.enable = true;
    man.enable = true;
    matplotlib.enable = true;
    mcfly.enable = true;
    # mercurial.enable = true; # config
    pandoc.enable = true;
    # password-store.enable = true;
    powerline-go.enable = true;
    #pyenv.enable = true;
    pylint.enable = true;
    pywal.enable = true;
    rbenv.enable = true;
    readline.enable = true;
    #ripgrep.enable = true;
    rtorrent.enable = true;
    # sagemath.enable = true; # oh my god 1 hour + build times and then it usually fails. if it's cached you're fine but on unstable it is just not always cached. even worse against master branch
    # ssh = {
    #   enable = true;
    #   enableDefaultConfig = true;
    # evaluation warning: user profile: You have set either `nixpkgs.config` or `nixpkgs.overlays` while using `home-manager.useGlobalPkgs`.
    #                     This will soon not be possible. Please remove all `nixpkgs` options when using `home-manager.useGlobalPkgs`.
    # evaluation warning: user profile: `programs.ssh` default values will be removed in the future.
    #                     Consider setting `programs.ssh.enableDefaultConfig` to false,
    #                     and manually set the default values you want to keep at
    #                     `programs.ssh.matchBlocks."*"`.
    # };
    starship.enable = true;
    swaylock.enable = true;
    taskwarrior = {
      enable = true;
      package = pkgs.taskwarrior3;
    };
    tealdeer.enable = true;
    terminator.enable = true;
    # termite.enable = true; # removed from nixpkgs: broken and unmaintained upstream
    #texlive.enable = true; # failed on wsl
    # thunderbird.enable = true;
    tiny.enable = true;
    #tmate.enable = true; # insecure in this nixpkgs: CVE-2018-19387, no release since 2019
    # tmux.enable = true;
    # vim-vint.enable = true;
    # vim.enable = true;
    # vscode.enable = true;
    wlogout.enable = true;
    zathura.enable = true;
    zellij.enable = true;
    zoxide.enable = true;
    # zplug.enable = true;
    nushell = {
      enable = true;
      environmentVariables = {
        NIXOS_OZONE_WL = "1";
        ELECTRON_OZONE_PLATFORM_HINT = "auto";
        EDITOR = "nvim";
        VISUAL = "nvim";
        #TERM = "kitty"; # alacritty";
      };
      shellAliases = {
        #switch = "sudo nixos-rebuild switch";
      };
      extraConfig = ''
        $env.config = {
          show_banner: false,
        }
      '';
    };
    #oils-for-unix.enable = true;
    obs-studio.enable = true;
    oh-my-posh.enable = true;
    #fish.enable = true;
    bat.enable = true;
    #zsh = {
    #  enable = true;
    #  oh-my-zsh = {
    #    enable = true;
    #    plugins = [
    #      "git"
    #      "python"
    #      "docker"
    #      "fzf"
    #    ];
    #    theme = "dpoggi";
    #  };
    #};
    htop = {
      enable = true;
      settings = {
        delay = 10;
        show_program_path = false;
        show_cpu_frequency = true;
        show_cpu_temperature = true;
        hide_kernel_threads = true;
        leftMeters = [
          "AllCPUs2"
          "Memory"
          "Swap"
        ];
        rightMeters = [
          "Hostname"
          "Tasks"
          "LoadAverage"
          "Uptime"
          "Systemd"
        ];
      };
    };
    tmux = {
      enable = true;
      # setw -g mouse on
    };
    password-store = {
      enable = true;
      settings = {
        PASSWORD_STORE_DIR = "$XDG_DATA_HOME/password-store";
      };
    };
  };
  home = {
    inherit stateVersion;
    shellAliases = {
      l = "exa";
      ls = "exa";
      cat = "bat";
    };
    sessionVariables = {
      EDITOR = "nvim";
      #TERM = "kitty"; # "alacritty" "xterm-256color"
      # PATH = "$HOME/bin:$PATH";
      NIXOS_OZONE_WL = "1"; # This variable fixes electron apps in wayland
      NIXPKGS_ALLOW_UNFREE = "1";
      # OpenCode feature flags (Drew, 2026-10-20): the interesting
      # experimental features. OPENCODE_EXPERIMENTAL is the umbrella
      # (code mode keys off it). NATIVE_LLM deliberately not set.
      # (OPENCODE_ENABLE_EXA / OPENCODE_ENABLE_PARALLEL removed 2026-10-23:
      # nothing in opencode reads them — see opencode #105.)
      OPENCODE_ENABLE_EXPERIMENTAL_MODELS = "1";
      OPENCODE_EXPERIMENTAL = "1";
      OPENCODE_EXPERIMENTAL_PARALLEL = "1";
      OPENCODE_EXPERIMENTAL_PLAN_MODE = "1";
      OPENCODE_EXPERIMENTAL_SCOUT = "1";
      OPENCODE_EXPERIMENTAL_WORKSPACES = "1";
      OPENCODE_EXPERIMENTAL_OXFMT = "1";
      OPENCODE_EXPERIMENTAL_EXA = "1";
      OPENCODE_EXPERIMENTAL_EVENT_SYSTEM = "1";
      # Single source for the local proxy ports (see the let block):
      # opencode.jsonc reads them via {env:...} substitution.
      CLEF_PROXY_PORT = toString clefProxyPort;
      AGY_PROXY_PORT = toString agyProxyPort;
      CODEX_PROXY_PORT = toString codexProxyPort;
      ANTIGRAVITY_LS_PROXY_PORT = toString antigravityLsProxyPort;
      DESIGN_CODE_PROXY_PORT = toString designCodeProxyPort;
      # The three below were missing: their {env:...} templates in
      # opencode.jsonc resolved to an empty port in the OpenChamber serve
      # (2026-10-25 audit) because the variables never reached its
      # environment. The session list must cover every let-block port.
      MUSE_PROXY_PORT = toString museProxyPort;
      COMMANDCODE_PROXY_PORT = toString commandcodeProxyPort;
      CLAUDE_CODE_PROXY_PORT = toString claudeCodeProxyPort;
      OPENCODE_EXPERIMENTAL_DISABLE_COPY_ON_SELECT = "1";
      # File watcher off by default (Drew, 2026-10-20): the working trees
      # are giant, and watching costs more than the live-external-edit
      # awareness it buys. Diffs are git-computed on demand either way.
      # (Name corrected 2026-10-25 per #698: opencode's server-process.ts
      # reads OPENCODE_DISABLE_FILEWATCHER; the EXPERIMENTAL_ name was
      # read nowhere, so the watcher was silently still on.)
      OPENCODE_DISABLE_FILEWATCHER = "1";
      # XDG_CACHE_HOME = "$HOME/.cache";
      # XDG_CONFIG_DIRS = "/etc/xdg";
      # XDG_CONFIG_HOME = "$HOME/.config";
      # XDG_DATA_DIRS = "/usr/local/share/:/usr/share/";
      # XDG_DATA_HOME = "$HOME/.local/share";
      # XDG_STATE_HOME = "$HOME/.local/state";
    };
    sessionPath = [
      "$HOME/.local/bin"
      "$HOME/.cache/.bun/bin"
    ];
    # pointerCursor = {
    #   package = pkgs.vanilla-dmz;
    #   name = "Vanilla-DMZ";
    #   gtk.enable = true;
    #   size = 24;
    #   x11.enable = true;
    # };
    file.".config/nixpkgs/config.nix".text = ''
      {
        allowUnfree = true;
      }
    '';
    packages =
      with pkgs;
      [
        libnotify
        (pkgs.writeShellScriptBin "clef-proxy" ''
          exec ${pkgs.nodejs}/bin/node ${../../bin}/clef-proxy.mjs "$@"
        '')
        (pkgs.writeShellScriptBin "agy-proxy" ''
          export PATH="$HOME/.gemini/bin:$HOME/.local/bin:$PATH"
          exec ${pkgs.nodejs}/bin/node ${../../bin}/agy-proxy.mjs "$@"
        '')
        (pkgs.writeShellScriptBin "codex-proxy" ''
          export PATH="/run/current-system/sw/bin:$HOME/.local/bin:$PATH"
          exec ${pkgs.nodejs}/bin/node ${../../bin}/codex-proxy.mjs "$@"
        '')
        (pkgs.writeShellScriptBin "antigravity-ls-proxy" ''
          export PATH="/run/current-system/sw/bin:$HOME/.gemini/bin:$HOME/.local/bin:$PATH"
          exec ${pkgs.nodejs}/bin/node ${../../bin}/antigravity-ls-proxy.mjs "$@"
        '')
        (pkgs.writeShellScriptBin "design-code-proxy" ''
          export PATH="/run/current-system/sw/bin:$HOME/.local/bin:$PATH"
          exec ${pkgs.nodejs}/bin/node ${../../bin}/design-code-proxy.mjs "$@"
        '')
        (pkgs.writeShellScriptBin "sync-models" ''
          export PATH="/run/current-system/sw/bin:$HOME/.gemini/bin:$HOME/.local/bin:$PATH"
          exec ${pkgs.nodejs}/bin/node ${../../bin/sync-models.mjs} "$@"
        '')
        (pkgs.writeShellScriptBin "update-models" ''
          export PATH="/run/current-system/sw/bin:$HOME/.gemini/bin:$HOME/.local/bin:$PATH"
          exec ${pkgs.nodejs}/bin/node ${../../bin/sync-models.mjs} "$@"
        '')
        (pkgs.writeShellScriptBin "opencode" ''
          if [ -z "$CLOUDFLARE_API_TOKEN" ] && [ -r "$HOME/.config/cloudflare/ai-inference-token" ]; then
            [ -r "$HOME/.config/cloudflare/account-id" ] && export CLOUDFLARE_ACCOUNT_ID="$(< "$HOME/.config/cloudflare/account-id")"
            [ -r "$HOME/.config/cloudflare/gateway-id" ] && export CLOUDFLARE_GATEWAY_ID="$(< "$HOME/.config/cloudflare/gateway-id")"
            export CLOUDFLARE_API_TOKEN="$(< "$HOME/.config/cloudflare/ai-inference-token")"
          fi
          if [ -z "$JULES_API_KEY" ] && [ -r "$HOME/.config/jules/api-key" ]; then
            export JULES_API_KEY="$(< "$HOME/.config/jules/api-key")"
          fi

          # Opportunistic background model sync (max once every 24h, detached)
          _CACHE_DIR="$HOME/.cache"
          [ -n "$XDG_CACHE_HOME" ] && _CACHE_DIR="$XDG_CACHE_HOME"
          _SYNC_STAMP="$_CACHE_DIR/sync-models.last"
          if [ ! -f "$_SYNC_STAMP" ] || [ -n "$(find "$_SYNC_STAMP" -mtime +1 2>/dev/null)" ]; then
            mkdir -p "$_CACHE_DIR"
            touch "$_SYNC_STAMP"
            (
              export PATH="/run/current-system/sw/bin:$HOME/.gemini/bin:$HOME/.local/bin:$PATH"
              if command -v sync-models >/dev/null 2>&1; then
                sync-models >/dev/null 2>&1
              elif [ -x "$HOME/code/bin/sync-models.mjs" ]; then
                ${pkgs.nodejs}/bin/node "$HOME/code/bin/sync-models.mjs" >/dev/null 2>&1
              fi
            ) &
          fi

          exec ${inputs.opencode.packages.${system}.opencode}/bin/opencode "$@"
        '')
        #
        #         dog
        #         felix
        #         figlet/*
        #         gcc
        #         helix
        #         hex
        #         lolcat
        #         lolcat*/*/
        #         nodePackages.prettier
        #         oh-my-zsh
        #         polybar
        #         python-debug
        #         rofi
        #         tldr
        #         waybar-hyprland-git
        #       swayidledd
        #     configure-gtk
        #     dbus-sway-environment
        #     hyprland
        #     inputs.hyprwm-contrib.packages.${system}.grimblast
        #  1history
        #  astro
        #  cakawka
        #  calculator
        #  cicada
        #  counts
        #  cpc
        #  delicate
        #  dtrace
        #  dua-cli
        #  dust du-dust above
        #  floki
        #  frum
        #  hashguard
        #  kani-verifier
        #  legdur
        #  lemmy
        #  medic
        #  mrml
        #  nat
        #  notty
        #  opentelemetry
        #  oreboot
        #  pepper
        #  pleco
        #  printfn
        #  qsv
        #  rip
        #  rustodon
        #  stringsext
        #  teehee
        #  tv-renamer
        #  voila
        #  weld
        #  xi
        #  zh
        # Bash
        # Command Shells
        # Core Packages
        # Dart
        # Development
        # Elixir
        # Erlang
        # Files
        # Haskell
        # Joke/*s
        # Language Servers
        # Lua
        # Media
        # My Packages
        # My Proprietary Packages
        # Nix
        # Overview
        # Programming Languages
        # Python
        # QT
        # Rust CLI Tools! I love rust.
        # Standard Packages
        # Telescope tools
        # These are so intellij file watchers has something to use
        # Typescript
        # Web (ESLint, HTML, CSS, JSON)
        # Xorg Stuff :-(
        # bandwhich # isn't working right?
        # calibre
        # cliphist
        # egui_graphs
        # fenix
        # frolic
        # hot-lib-reloader
        # https://github.com/Inlyne-Project/inlyne/issues/356
        # https://github.com/NixOS/nixpkgs/issues/332957
        # hyprland-share-picker
        # inlyne # rust 1.80
        # intelli-shell
        # libsForQt6.qt6.qtwayland
        # lua
        # mlocate # shadowed by plocate
        # neovim
        # oil # try again later
        # plotlib
        # plotly
        # python.pkgs.pip
        # qt5-wayland
        # qt6-wayland
        # ripgrep-all # regression cannot find hello 26 times 2023-08-19
        # ripsecrets
        # rmesg # unknown
        # rpn
        # rustfix
        # soup
        # sqlitecpp
        # tldr # shadowed by tealdeer
        # todo figure out how to use sway
        # trustfall
        # vim-racer
        # xd # i don't know what this is
        ## Desktop Environments
        ## Go
        ## Libraries
        ## Programs
        ## Rust
        ## Window Managers
        ## block ick
        ## endblock ick
        #awesome
        #cargo-graph
        #cinnamon.cinnamon-desktop
        #duckdb # long compile todo
        #dust # abandoned
        #element-desktop # build time long, electron bad
        #eww-wayland
        #exa
        #eza # exa # ls
        #fh # ffi parse failure
        #fprint
        #gnupg
        #monero-gui
        #neofetch
        #neovim
        #nixfmt
        #nodePackages.pyright
        #nushell
        #oil # oil is python oils-for-unix is cpp
        #pinentry
        #pinentry-qt
        #plasma5Packages.kdenlive # build failures? maybe need plasma6?
        #pyright
        #qtcreator
        #rnix-lsp
        #signal-desktop
        #skypeforlinux
        #slack
        #tabnine
        #tdesktop
        #terraform
        #tor-browser-bundle-bin
        #tp-note # unknown
        #trash-cli
        #vim
        #vimPlugins.cmp-tabnine
        #vimPlugins.coc-tabnine
        #vimPlugins.copilot-cmp
        #vimPlugins.nvim-cmp
        #vimPlugins.nvim-treesitter-parsers.toml
        #vimPlugins.nvim-treesitter-parsers.typescript
        #vimPlugins.tabnine-vim
        #vimPlugins.telescope-zoxide
        #vimPlugins.vim-prettier
        #vimPlugins.vim-toml
        #vimPlugins.zoxide-vim
        #vscode
        #vscode-insiders
        #waybar-hyprland
        #ytop # abandoned
        #zoom-us
        acpi
        acpitool
        # adwaita-icon-theme # default gnome cursors
        #alacritty
        #alacritty # gpu accelerated terminal
        alsa-lib
        amp
        any-nix-shell
        arandr
        atuin
        audacity
        autojump
        autorandr
        awscli
        latestCli.backblaze-b2
        bacon
        bat
        bat # cat
        #beam.packages.erlang.elixir-ls
        erlang-language-platform # erlang-ls # https://github.com/NixOS/nixpkgs/pull/448119
        beep
        bemenu # wayland clone of dmenu
        bingrep
        #bitwarden
        bitwarden-cli
        bitwarden-desktop
        bitwarden-menu
        black
        blink1-tool
        blueman
        bluez
        bluez-tools
        bottom
        brave
        brig
        brightnessctl
        brillo
        broot
        bspwm
        btop
        cachix
        cargo
        cargo-audit
        cargo-binstall
        cargo-crev
        cargo-geiger
        cargo-wipe
        cava
        ccls
        celluloid
        charm
        charm-freeze
        choose
        latestCli.cloudflared
        cmake
        cmatrix
        conform
        consul
        coreutils
        # OCI/registry tooling -- covers ghcr.io ("github oci") workflows.
        # skopeo for copy/inspect across registries, crane for fast scripted
        # push/pull, cosign for keyless signing via GitHub OIDC, oras for
        # non-image artifacts (Helm charts, SBOMs), regctl for introspection.
        cosign
        # provides crane/gcrane/krane
        latestCli.go-containerregistry
        cpufetch
        curl
        #dart
        dash
        delta # better diff
        deno
        difftastic
        direnv
        discord
        dive
        # diskonaut # https://github.com/NixOS/nixpkgs/pull/376644
        dmenu
        dnsutils
        docker-compose
        # dogdns # dns for dogs
        kdePackages.dolphin
        dprint
        # dracula-theme # gtk theme # removed from nixpkgs: depended on gtk-engine-murrine (GTK2, unmaintained)
        drill
        dust
        dua
        dunst
        dwm
        elinks
        elmPackages.elm-format
        endlessh
        espeak
        eva
        eww
        eza
        fastmod
        fblog
        fclones
        fd
        fd # replace find
        feh
        fend
        ffsend
        flameshot
        flatpak
        latestCli.flyctl
        fnm
        #fontconfig
        # fontfinder
        freetype
        fselect
        furtherance
        fw
        fzf
        # same derivation as programs.gh.package above; buildEnv dedups
        latestCli.gh
        gimp
        git
        git-absorb
        git-cliff
        git-crypt
        diff-so-fancy
        github-desktop
        gitui
        glib
        glib # gsettings
        glibc
        gnugrep
        gnumake
        gnupg
        gnused
        go
        gparted
        gptman
        grex # ya grep
        grim
        grim # screenshot functionality
        gthumb
        gtklock
        haskellPackages.haskell-language-server
        hck
        helix # neovim 2
        hexyl
        himalaya
        html-tidy
        htmlq
        htop
        htop # top for humans
        huniq
        # Honeycomb's tail-sampling proxy. Directly relevant on the free plan:
        # the quota is 20M events/month and every span, SpanEvent and Link counts
        # as one event, so a chatty service burns it fast. Sampling before send
        # is the supported lever -- sampled-away events are not counted.
        #
        # The nixpkgs build also ships a second binary called plain `convert`
        # (a v1->v2 config migrator) which collides with ImageMagick's `convert`
        # and fails the home-manager buildEnv. The prebuilt package here installs
        # only `refinery`, so the collision is gone by construction.
        latestCli.honeycomb-refinery
        # Honeycomb ships no general CLI (honeyvent/honeytrigger are archived
        # upstream and there is no `hny`). honeymarker handles deploy markers and
        # is the only non-archived one left.
        honeymarker
        hyperfine
        hyprdim
        hyprland-autoname-workspaces
        wl-gammactl
        hyprland-per-window-layout
        hyprland-protocols
        hyprpaper
        imagemagick
        intel-gpu-tools
        ion
        kubo # ipfs
        jack2
        #jetbrains-mono
        jless
        jq
        jql
        just
        k9s
        kalker
        # kdash # removed from nixpkgs: upstream tag changes broke the source derivation
        kibi
        kickoff
        killall
        #kitty
        kondo
        krabby
        lagrange
        lapce
        lazygit
        lazygit # command line git ui
        lefthook
        lemmeknow
        less
        lf
        dysk # lfs
        git-lfs
        libnotify
        #libsForQt5.polkit-kde-agent
        kdePackages.polkit-kde-agent-1
        qt5.qtwayland # was libsForQt5.qt5.qtwayland
        kdePackages.yakuake
        libtool
        libva-utils
        libverto
        #licensor # https://github.com/NixOS/nixpkgs/issues/141368
        # light
        lld
        #lmms # https://github.com/NixOS/nixpkgs/issues/450908 # https://github.com/NixOS/nixpkgs/pull/377643
        #loc
        lsd
        lua-language-server
        lxsession
        macchina
        # mako # notification system developed by swaywm maintainer
        mangohud
        mask
        mcfly
        mdbook
        mdcat
        miniserve
        mkcert
        monolith
        mosh
        mpv
        nano
        navi
        ncspot
        # neofetch # sysinfo
        fastfetch
        neovim
        networkmanager
        networkmanagerapplet
        nfs-utils
        nickel
        nil
        ninja
        # nitrogen # removed from nixpkgs: depended on deprecated gtk2/gtkmm2
        nix-init
        nix-melt
        nixfmt # nixfmt-rfc-style is now just pkgs.nixfmt
        nixpkgs-fmt # ??
        bash-language-server # nodepackages removed 2026-04-03
        eslint # nodepackages removed 2026-04-03
        prettier # nodepackages  nodepackages removed?? # removed 2026-04-03
        #nodePackages.prettier-plugin-toml
        #nodePackages.typescript
        #nodePackages.typescript-language-server
        #nodePackages.vercel
        #nodePackages.vscode-langservers-extracted
        #nodePackages.wrangler
        #nodejs
        #nodejs-18_x
        nomacs
        nomino
        nsh
        nurl
        nwg-displays
        nwg-dock-hyprland
        #oh-my-fish
        openconnect
        openssl
        # Oracle Cloud Infrastructure CLI; provides the `oci` command.
        oci-cli
        # OpenTelemetry. OpenTracing (which it superseded) was archived in 2022;
        # OTLP is what Honeycomb actually ingests.
        #   otel-cli  -- emit spans from shell scripts, and the quickest way to
        #               smoke-test an ingest key end to end.
        #   collector -- contrib build, since the vendor-specific exporters and
        #               processors are not in the core distribution.
        latestCli.opentelemetry-collector-contrib
        latestCli.oras
        ormolu
        otel-cli
        latestCli.ovhcloud-cli
        ouch
        packer
        pastel
        pavucontrol
        pciutils
        pgfplots
        picom
        pinentry-all # TODO: consider pinentry though gnupg services service, also consider whether this should be global and not for 'user'
        pinentry-rofi
        pipewire
        pipr
        pkg-config
        please
        plocate
        pls
        polkit_gnome
        polybarFull
        powershell
        procps
        procs
        procs # replace proc
        pstree
        pueue
        pulseaudio
        pv
        pwgen
        python3
        #python3Full
        qt5.qmake
        qt5.qtwayland
        libsForQt5.qt5ct # qt5.* top-level attrs removed; pkg stays under libsForQt5
        qt6.qmake
        qt6.qtwayland
        qt6Packages.qt6ct
        qtractor
        racer
        ranger # midnight commander / file manager
        # rargs # https://github.com/NixOS/nixpkgs/issues/141368
        rbw
        latestCli.regctl
        restic
        ripgrep
        rmtrash # ctrl + z for rm
        rnr
        rofi-rbw
        rofi # -wayland
        rufo
        runiq
        rust-analyzer
        rustc
        rustdesk
        rustfmt
        sad
        sd
        shellcheck
        shellharden
        shfmt
        silver-searcher-ng # was silver-searcher, removed (pcre1)
        skim
        skopeo
        slack
        slurp
        slurp # screenshot functionality
        sops
        sox
        spotify
        sqlite
        st
        starship
        starship # replacement prompt (not shell)
        statix
        stdenv
        steam
        sway
        swaycons
        swayidle
        swaylock
        swaynotificationcenter
        awww # renamed from swww
        sxhkd
        synergy
        systeroid
        tealdeer # ya tldr
        #terminus-nerdfont
        pay-respects # thefuck
        tidy-viewer
        tidyp
        tig # command line git
        tiny
        tmux
        tmuxPlugins.continuum
        tmuxPlugins.resurrect
        toilet
        tokei
        tokei # this gives language stats about a repo
        topgrade
        # t3code splits across two sources on purpose.
        #
        # latestCli.t3 is the server/CLI from upstream's release tarball at
        # 0.1.2 (Orchestrator V2), whose headless/Tailscale path is exactly
        # what systemd.user.services.t3code uses below.
        latestCli.t3
        # The Electron desktop GUI is a separate AppImage asset upstream,
        # not in that tarball. It used to be taken from nixpkgs (which was
        # on 0.0.45, the V1 line); it is now the local pkgs/t3code-desktop
        # package at the same 0.1.2 as the server -- bound as `t3code` in
        # the let block above, shadowing pkgs.t3code -- because a V1 app
        # refuses a V2 server and vice versa. The desktop package also
        # ships its own `t3` binary, which would collide with the above,
        # so only t3code-desktop and its desktop entry are taken from it.
        (runCommand "t3code-desktop-${t3code.version}"
          {
            meta = t3code.meta // {
              mainProgram = "t3code-desktop";
            };
          }
          ''
            mkdir -p "$out/bin"
            ln -s ${t3code}/bin/t3code-desktop "$out/bin/t3code-desktop"
            if [ -d ${t3code}/share ]; then
              mkdir -p "$out/share"
              cp -r ${t3code}/share/. "$out/share/"
            fi
          ''
        )
        transmission_4-gtk
        trash-cli
        tree
        tree-sitter
        treefmt
        trippy
        # turso-cli (above) is the Turso platform CLI and provides `turso`.
        # This attr is the separate local SQL shell and provides `tursodb`.
        latestCli.turso
        latestCli.turso-cli
        udev
        universal-ctags # for nvim nvchad custom
        unzip
        # vanilla-dmz
        variety
        vault
        vaultwarden
        vdpauinfo
        vlc # build time long, vlc good, try again later
        volta
        w3m
        warp
        watchexec
        wdisplays
        wdisplays # tool to configure displays
        wezterm
        wget
        whois
        wireplumber
        wl-clipboard
        wl-clipboard # wl-copy and wl-paste for copy/paste from stdin / stdout
        wofi
        # nixpkgs 4.94.0 vs npm 4.147.0 -- 53 minor versions behind, the
        # largest gap in this config. See pkgs/wrangler.
        (pkgs.callPackage ../../pkgs/wrangler { })
        # cf is Wrangler's successor (open beta 2026-09-28, not in
        # nixpkgs). Standing direction 2026-10-20: prefer cf for
        # Cloudflare work; wrangler stays for existing projects until
        # migrated (`cf migrate`). See pkgs/cf.
        (pkgs.callPackage ../../pkgs/cf { })
        wtf
        wttrbar
        xclip
        xcp
        xdg-desktop-portal
        # Provided system-wide (and version-matched to the master compositor) by
        # programs.hyprland.portalPackage in nixos/hyprland. Installing the
        # nixpkgs 0.56.2-built copy here only put a skewed binary on PATH.
        # xdg-desktop-portal-hyprland
        xdg-utils
        xdg-utils # for opening default programs when clicking links
        thunar # moved to top-level from xfce.thunar
        xh
        libx11 # xorg set deprecated
        # xorg.libXcursor
        xournalpp # xournal
        #xsv # https://github.com/NixOS/nixpkgs/issues/141368
        yad
        yarn
        ydotool
        yt-dlp # youtube-dl
        zee
        zellij
        zellij # tmux
        zig
        zip
        zlib
        zls
        zoom-us
        zoxide
        zstd
        zsv
        zulip
        zulip-term
      ]
      ++ [
        ## R
        (pkgs.rWrapper.override {
          packages = with pkgs.rPackages; [
            dplyr
            xts
            ggplot2
            reshape2
          ];
        })
        #(pkgs.rstudioWrapper.override {
        #  packages = with pkgs.rPackages; [
        #    dplyr
        #    xts
        #    ggplot2
        #    reshape2
        #    rstudioapi
        #  ];
        #})
      ];
  };
}
