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
  # clefProxyPort (18742) retired 2026-11-04 (opencode#1410):
  # the OpenChamber classifier rides the in-fork facade's
  # /v1/systemone (127.0.0.1:13420); the port is free.
  codexProxyPort = 18744;
  museProxyPort = 18747;
  # commandcodeProxyPort (18748) and claudeCodeProxyPort (18749)
  # retired 2026-11-06 (Drew, proxy-infra decommission): both
  # providers were repointed to the fork's native wire-emulation
  # providers on 2026-11-05 (code#6117, opencode#1749), so the CLI
  # proxies had no consumers left; their lets, units, session
  # variables, bin programs and guard rows were removed in one
  # change. Both ports are free.
  # opencodeRouteProxyPort (18750) retired 2026-11-04
  # (opencode#1153): OpenClaw repointed to the server facade
  # (127.0.0.1:13420/v1); the port is free.
  # antigravityDirectProxyPort (18751) retired 2026-11-04 (opencode#1414):
  # the in-fork native provider replaced the door; the port is free.
  # Atlas search service port (code#5350 step 5, #5355): the fleet
  # inventory entry for 18760. Unlike the proxy lets above, this
  # value is NOT plumbed into a unit's Environment — it lives in
  # the service's own config (~/.local/state/atlas-search/
  # service.json, written by `atlas-search init`) and in the Rust
  # defaults (server listen, CLI service URL), all 18760. The let
  # exists so the reservation is recorded where the fleet looks
  # for one; verified free against this file, the proxy-port
  # guard (.github/scripts/check-proxy-port-fallbacks.sh), and
  # the live listener table on 2026-11-04.
  atlasSearchPort = 18760;
  # The atlas-search package (pkgs/atlas-search), built with the
  # toolchain pinned by its own rust-toolchain.toml (channel
  # 1.99.0) through the fleet's rust-overlay lock — the pkgs/id
  # pattern (cf. rustToolchain in nixos/environment/default.nix).
  # Used as the atlas-search unit's ExecStart below and, since
  # the step-6 cutover (#5356), as a home package below so the
  # Rust binary is the resolved `atlas-search` CLI (the Python
  # CLI is kept alongside as `atlas-search-py`). It was kept
  # out of home.packages through step 5 so the Python CLI
  # stayed resolved until the cutover said otherwise.
  atlasSearchToolchain = (pkgs.extend (import inputs.id-rust-overlay)).rust-bin.fromRustupToolchainFile ../../pkgs/atlas-search/rust-toolchain.toml;
  atlasSearch = pkgs.callPackage ../../pkgs/atlas-search {
    rustPlatform = pkgs.makeRustPlatform {
      cargo = atlasSearchToolchain;
      rustc = atlasSearchToolchain;
    };
  };
  # The fleet multicall package (pkgs/fleet), built with the
  # toolchain pinned by its own rust-toolchain.toml — the
  # atlas-search pattern above. Used as the broker units'
  # ExecStart below (`fleet serve <role>`); deliberately not in
  # home.packages — the resolved `repo-queue` on PATH stays the
  # standalone artifact until the proxy step's shim dir (design
  # note §4) supersedes the hand-maintained symlinks.
  fleetToolchain = (pkgs.extend (import inputs.id-rust-overlay)).rust-bin.fromRustupToolchainFile ../../pkgs/fleet/rust-toolchain.toml;
  fleetPkgs = pkgs.callPackage ../../pkgs/fleet {
    rustPlatform = pkgs.makeRustPlatform {
      cargo = fleetToolchain;
      rustc = fleetToolchain;
    };
    sourceRev = inputs.self.rev or "dirty";
  };
  # Upstream prebuilt release binaries for tools where nixpkgs trails upstream.
  # See pkgs/latest-cli/default.nix for the rationale, the tradeoffs, and the
  # list of things deliberately NOT bumped.
  latestCli = pkgs.callPackage ../../pkgs/latest-cli { };
  # Vendored from nixpkgs master; see the header in that file for why. Provides
  # `agy_acp_server`, which t3code's AcpRegistryDriver can drive.
  antigravity-acp = pkgs.callPackage ../../pkgs/antigravity-acp { };
  # The hub build ships the language_server binary the retired
  # (2026-10-30) antigravity-ls proxy drove. Same
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
  # Hidden-shell auth hook (1Password service account, Cloudflare).
  # Same derivation nixos/agent-auth uses, so the BASH_ENV paths agree.
  agent-shell-env = pkgs.callPackage ../../pkgs/agent-shell-env { };
  # mcpx MCP gateway and Code Mode execution runner.
  mcpx = pkgs.callPackage ../../pkgs/mcpx/package.nix { };
  # The cascade's opencode build -- openchamber's nested opencode
  # input, which is exactly the derivation services.opencode.package
  # defaults to (the fork's module wrapper mkDefaults the fork
  # flake's own packages.<system>.opencode). Named once here: the
  # wrapper below execs it, and the wrapped package joins over it.
  opencodeCascade = inputs.openchamber.inputs.opencode.packages.${system}.opencode;
  # Drew's opencode wrapper -- formerly a standalone home.packages
  # entry (writeShellScriptBin "opencode"), logic unchanged:
  # Cloudflare env top-up and the stamp-gated sync-models
  # kick. The one change is the exec target: the cascade build, not
  # this repo's root opencode input (a second fork build, so CLI and
  # service ran different fork commits).
  opencodeWrapper = pkgs.writeShellScript "opencode" ''
    if [ -z "$CLOUDFLARE_API_TOKEN" ] && [ -r "$HOME/.config/cloudflare/ai-inference-token" ]; then
      [ -r "$HOME/.config/cloudflare/account-id" ] && export CLOUDFLARE_ACCOUNT_ID="$(< "$HOME/.config/cloudflare/account-id")"
      [ -r "$HOME/.config/cloudflare/gateway-id" ] && export CLOUDFLARE_GATEWAY_ID="$(< "$HOME/.config/cloudflare/gateway-id")"
      export CLOUDFLARE_API_TOKEN="$(< "$HOME/.config/cloudflare/ai-inference-token")"
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

    exec ${opencodeCascade}/bin/opencode "$@"
  '';
  # The cascade opencode with the wrapper as its bin/opencode.
  # symlinkJoin over the cascade package: the rest of its tree --
  # above all bin/mcpx, the cascade mcpx symlink the code#5027
  # settlement relies on -- rides along untouched. This is what
  # services.opencode.package points at (below), so the systemd user
  # service and the home profile share this one derivation.
  opencodeWrapped = pkgs.symlinkJoin {
    name = "${opencodeCascade.name}-wrapped";
    paths = [ opencodeCascade ];
    postBuild = ''
      rm $out/bin/opencode
      install -Dm755 ${opencodeWrapper} $out/bin/opencode
    '';
  };

  # Brave with structural CDP (code#5924 follow-up): Brave is
  # Chromium-family, so it takes the same --remote-debugging flags
  # as Helium (19222) and system Chromium (19224); Brave gets
  # 19223, loopback-only. Same wrapper shape as
  # wrapElectronWithFlags in nixos/environment (binaries wrapped,
  # desktop entries repointed at the wrappers) so every launch
  # path -- shell or launcher -- carries the flags. Caller-passed
  # flags come after wrapper flags and win.
  braveCdp = pkgs.symlinkJoin {
    name = "${pkgs.brave.name}-cdp";
    paths = [ pkgs.brave ];
    nativeBuildInputs = [ pkgs.makeWrapper ];
    postBuild = ''
      for bin in "$out"/bin/*; do
        [ -L "$bin" ] || continue
        target=$(readlink -f "$bin")
        [ -x "$target" ] || continue
        rm "$bin"
        makeWrapper "$target" "$bin" \
          --add-flags "--remote-debugging-address=127.0.0.1" \
          --add-flags "--remote-debugging-port=19223"
      done

      if [ -d "$out/share/applications" ]; then
        for f in "$out"/share/applications/*.desktop; do
          [ -e "$f" ] || continue
          src=$(readlink -f "$f")
          base=$(basename "$f")
          rm "$f"
          sed "s|${pkgs.brave}/bin/|$out/bin/|g" "$src" > "$out/share/applications/$base"
        done
      fi
    '';
    meta = (pkgs.brave.meta or { }) // {
      mainProgram = pkgs.brave.meta.mainProgram or null;
    };
  };
  # Firefox with structural WebDriver BiDi (code#5924 follow-up):
  # Firefox has no CDP; --remote-debugging-port starts its
  # WebDriver BiDi remote agent (binds 127.0.0.1 itself; default
  # Host/Origin policy kept -- see nixos/environment for the full
  # port table: 19225 Firefox BiDi, 19226 Zen BiDi). Home-manager
  # installs this as the programs.firefox package below; the
  # NixOS side installs the same wrapper via
  # environment.systemPackages (nixos/environment firefoxBidi).
  firefoxBidi =
    let
      wrapBidi =
        pkg:
        let
          joined = pkgs.symlinkJoin {
            name = "${pkg.name}-bidi";
            paths = [ pkg ];
            nativeBuildInputs = [ pkgs.makeWrapper ];
            postBuild = ''
              for bin in "$out"/bin/*; do
                [ -L "$bin" ] || continue
                target=$(readlink -f "$bin")
                [ -x "$target" ] || continue
                rm "$bin"
                makeWrapper "$target" "$bin" \
                  --add-flags "--remote-debugging-port=19225"
              done
            '';
            meta = (pkg.meta or { }) // {
              mainProgram = pkg.meta.mainProgram or null;
            };
          };
        in
        joined
        // {
          # home-manager's firefox module (mkFirefoxModule) treats a
          # wrapped package as reconfigurable only through its
          # `override`: it probes `lib.functionArgs package.override`
          # for `cfg` / `appDataDir` and calls it with an override
          # function merging cfg, extraPolicies and pkcs11Modules.
          # A bare symlinkJoin has no `override`, which broke the HM
          # evaluation of main in code#6156. Forward the underlying
          # package's override -- re-applying the BiDi wrap to its
          # result, with the override's exact function-args shape --
          # so the module behaves exactly as it does for pkgs.firefox.
          override = lib.setFunctionArgs (args: wrapBidi (pkg.override args)) (lib.functionArgs pkg.override);
        };
    in
    wrapBidi pkgs.firefox;
  # The environment the mcpx daemon runs with (services.mcpx.environment
  # below; the module renders it into the unit).
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
  # The daemon's configuration (the former ~/code/.mcpx.json), and the
  # config file the mcpx module renders from it: reproduced here so
  # the deploy key below tracks the daemon's real configuration
  # without this file needing the module system's `config`. (With an
  # empty `servers` the module renders
  # settings // { mcpServers = settings.mcpServers; }, so the two
  # writeText results are the same store path.)
  mcpxSettings = import ./mcpx-settings.nix;
  mcpxConfigRendered = pkgs.writeText "mcpx-config.json" (builtins.toJSON mcpxSettings);
  # The deploy key for the handover in home.activation.mcpxDeploy
  # below: a switch that changes the daemon, its configuration or its
  # environment hands the running daemon over, so the key covers
  # exactly those three.
  mcpxKey = deployKey "mcpx" [
    mcpx
    mcpxConfigRendered
    (builtins.toJSON mcpxEnvironment)
  ];
  # A unit is replaced at switch time only when its start command or binaries change.
  deployKey = name: parts: "${name}:${builtins.concatStringsSep " " (map toString parts)}";
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
      package = inputs.nixpkgs-unstable.legacyPackages.${pkgs.system}.activitywatch;
    };
  };
  # Every opencode on this machine is the 2.x fork (CLI and server), whose
  # canonical config keys are the v2 `plugins` / `agents`: the v1 `plugin` /
  # `agent` spellings are decoded as legacy and migrated in memory (the
  # fork's packages/core/src/config/normalize.ts). OpenChamber also writes
  # native v2 only. The 1.x CLI that needed the v1 spellings (it ignored
  # `plugins` entirely) is retired, so there is nothing left to protect by
  # converting back to v1 — that conversion is what made the keys flap on
  # every OpenChamber launch.
  #
  # Normalise any v1 spelling that lands in the file (hand edits, older
  # tools) to v2. The script is idempotent and refuses to write anything
  # it cannot re-parse. It is deliberately NOT watched by a path unit
  # right now — see the note under the service.
  systemd.user.services.opencode-config-normalize = {
    Unit = {
      Description = "Normalise opencode config to v2 plugin/agent keys";
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
  # No systemd.user.paths.opencode-config-normalize until the fleet
  # switch lands the v2 normaliser (2026-11-03): the normaliser in the
  # RUNNING generation (built 2026-11-01, before #4594 taught
  # pkgs/opencode-config-normalize to emit only v2) still writes the v1
  # spellings, and a declared path unit re-arms on every home-manager
  # activation — the runtime stops applied during the #5094 adoption
  # were silently undone and the tracked config flapped back to v1.
  # Until the switch, the service above runs only when started
  # deliberately (systemctl --user start opencode-config-normalize).
  # Restore the path unit in the same change that verifies the switched
  # generation carries the v2 normaliser:
  #   systemd.user.paths.opencode-config-normalize = {
  #     Unit.Description = "Watch opencode config for v1 plugin/agent keys";
  #     Path = {
  #       PathChanged = "%h/.config/opencode/opencode.jsonc";
  #       Unit = "opencode-config-normalize.service";
  #     };
  #     Install.WantedBy = [ "default.target" ];
  #   };

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
  # Fleet proxies (#2104): automatic user services, one per proxy, each
  # WantedBy default.target so it starts with the user session. ExecStart
  # runs the same checkout entry points the proxy-supervisor ran
  # (~/code/bin/*.mjs and the *-start.sh wrappers, via /run/current-system
  # node/bash, exactly as the supervisor's systemd-run invocations did),
  # so a proxy's code is the live checkout build, not a nix-store copy.
  # The proxy-supervisor's process ownership is retired in the same
  # change (its timer/service/config are removed below the proxy units):
  # exactly one owner per canonical port. Consumers (openchamber,
  # openclaw-gateway) Wants these units with Wants+After soft semantics.
  # Clef door retired 2026-11-04 (opencode#1410): the OpenChamber
  # classifier's endpoint file (~/.config/openchamber/
  # classifier-endpoint.json) names the in-fork facade's
  # /v1/systemone on the standing opencode server
  # (127.0.0.1:13420); live probe parity against the door
  # (byte-identical responses, including the shared upstream
  # Cloudflare auth failure) was proven before this removal.
  # bin/clef-proxy.mjs and its wrapper/test were removed in the
  # same change.

  # Antigravity Direct door retired 2026-11-04 (opencode#1414): the
  # provider block in .opencode/opencode.jsonc names the in-fork
  # native provider (@opencode/ai/providers/antigravity, opencode#1457)
  # under the same antigravity-direct id, and a zero-traffic window
  # on :18751 preceded this removal. bin/antigravity-direct.mjs and
  # its wrapper/test were removed in the same change; the agy CLI
  # remains the keyring credential owner and the native provider
  # refreshes in memory, in place.

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
          exec /run/current-system/sw/bin/node "$HOME/code/bin/codex-proxy.mjs"
        ''
      );
      Restart = "always";
      RestartSec = 3;
      Environment = [
        "CODEX_PROXY_PORT=${toString codexProxyPort}"
        "CODEX_PROXY_HOST=127.0.0.1"
        # Gate 1 cutover (opencode#1154): the in-fork codex credential
        # sync (opencode#1468) owns the token copy now; the door must
        # not overwrite the true JWT expiry with its synthetic one.
        "CODEX_PROXY_TOKEN_SYNC=0"
      ];
    };
    # No Install: codex-proxy.socket owns the listener and activates this
    # service on first connection (#1821). Enabling the service directly
    # as well would race the socket for the same port.
  };

  systemd.user.sockets.codex-proxy = {
    Unit.Description = "Codex proxy listener";
    Socket = {
      ListenStream = "127.0.0.1:${toString codexProxyPort}";
      Service = "codex-proxy.service";
    };
    Install.WantedBy = [ "sockets.target" ];
  };

  # Antigravity LS proxy retired (2026-10-30, Drew): its unit definition
  # stood here. The language-server door and the agy CLI-spawn door are
  # both replaced by antigravity-direct (above codex-proxy), which speaks
  # the CLI's wire protocol with no spawned processes. The definitions
  # are removed, not commented out, so the switch stops the services and
  # they stay off. bin/antigravity-ls-proxy.mjs and bin/agy-proxy.mjs
  # remained in the repo as history until the 2026-11-06 proxy-infra
  # decommission removed them; the matching opencode providers sit in
  # disabled_providers. To undo: restore the two unit blocks, the two
  # bin programs and the agyProxyPort/antigravityLsProxyPort lets from
  # git history, drop the session-variable removals below, and
  # re-point the plan seats in opencode.jsonc back from
  # antigravity-direct.

  # Design Code proxy removed (2026-11-03, Drew): its unit definition
  # stood here. Drew removed the design persona routes and the whole
  # design-code machinery — "the only design thing I have is opendesign"
  # — so the :18746 door has no consumers left: the persona routes are
  # removed from opencode.jsonc in the same pass, and the opendesign
  # provider/pool (a different thing, kept) never used this door. The
  # definition is removed, not commented out, so the switch stops the
  # service and it stays off. bin/design-code-proxy.mjs remained in
  # the repo as history until the 2026-11-06 proxy-infra decommission
  # removed it; its port let and session variable were removed with
  # the unit. To undo: restore this unit block, the bin program, the
  # designCodeProxyPort let, and the DESIGN_CODE_PROXY_PORT session
  # variable from git history, and re-add the config-side machinery.

  # Proxy supervisor retired (#2104, 2026-10-28): the proxy-supervisor
  # oneshot service, its 2-minute timer, and the supervisor config that
  # used to live here are removed. The per-proxy units in this file are
  # now the automatic services (Install.WantedBy default.target) and the
  # sole owners of the canonical ports; nothing may respawn the old
  # transient fps-* units. bin/proxy-supervise.mjs remains in the repo
  # as history, uninvoked. Its endpoint bookkeeping
  # (~/.local/state/<proxy>/endpoint.json) is likewise retired: the
  # files on disk are stale records with no writer, and service state
  # is read from the units themselves (systemctl --user show).
  # The supervisor's hash-keyed A/B roll behaviour is deliberately NOT
  # reproduced: a switch that changes a unit's ExecStart restarts that
  # proxy, per the normal flake service pattern (cf. mcpx/openchamber).

  # CLI proxy workroot (muse): the muse unit below points its
  # MUSE_PROXY_WORKDIR override at ~/.local/state/cli-proxies/muse,
  # so runtime session state lives outside the checkout (the script's
  # default, bin/work inside ~/code, dirtied the primary checkout
  # with every session and blocked pm-git-sync). The commandcode and
  # claude-code units that shared the cli-proxies root (#2785, #3067)
  # are retired below; their state subtrees are left in place.
  #
  # Command Code proxy retired 2026-11-06 (Drew, proxy-infra
  # decommission): its unit definition stood here. The provider was
  # repointed to the fork's native wire-emulation provider on
  # 2026-11-05 (code#6117, opencode#1749), so the :18748 CLI bridge
  # had no consumers left; its let, unit, session variable,
  # bin/commandcode-proxy.mjs, its start script and its test were
  # removed in the same change. The port is free.
  #
  # Claude Code proxy retired 2026-11-06 (Drew, proxy-infra
  # decommission): its unit definition stood here. Same story: the
  # provider was repointed to the fork's native wire-emulation
  # provider on 2026-11-05 (code#6117, opencode#1749) and re-enabled
  # as a native provider on 2026-11-06; the :18749 CLI bridge's let,
  # unit, session variable, bin/claude-code-proxy.mjs and its test
  # were removed in the same change. The port is free.

  # Muse (Meta) proxy (#2104): also previously supervisor-config only.
  # Runs the checkout start script, the supervisor's exact invocation;
  # the script owns its environment (including keeping META_API_KEY out).
  # Enrolled here so retiring the supervisor does not strand :18747 --
  # the redesign leaves exactly one owner for every proxy port.
  #
  # Direct mode (#2753, 2026-10-30): bin/muse-proxy.mjs defaults to
  # exchanging the Muse CLI account token for the plan's minted key
  # and serving the OpenCode muse-direct provider (declared in
  # .opencode/opencode.jsonc, the sole enabled Meta door) over the
  # Responses API; the CLI harness remains only as a fallback. The
  # credential is read at runtime from ~/.config/muse/auth.json --
  # it is never part of this config. Direct mode changes nothing
  # about this unit: same port, same start script, same env contract.
  systemd.user.services.muse-proxy = {
    Unit = {
      Description = "Muse (Meta) Local Proxy Bridge for OpenCode";
      After = [ "network-online.target" ];
      Wants = [ "network-online.target" ];
    };
    Service = {
      ExecStart = toString (
        pkgs.writeShellScript "muse-proxy-start-unit" ''
          exec /run/current-system/sw/bin/bash "$HOME/code/bin/muse-proxy-start.sh"
        ''
      );
      Restart = "always";
      RestartSec = 3;
      Environment = [
        "MUSE_PROXY_PORT=${toString museProxyPort}"
        "MUSE_PROXY_HOST=127.0.0.1"
        "MUSE_PROXY_WORKDIR=%h/.local/state/cli-proxies/muse"
      ];
    };
    Install.WantedBy = [ "default.target" ];
  };

  # Route proxy retired 2026-11-04 (opencode#1153): the door's
  # only live consumer, OpenClaw's opencode-route provider, was
  # repointed to the OpenCode server facade (127.0.0.1:13420/v1)
  # and verified live (four-part proof on opencode#1146: all five
  # route ids answered through the direct path, a workhorse turn
  # completed with this unit stopped and :18750 refusing
  # connections). Demotion lives in the fork (routes.settings
  # .demotion + ledger counts, opencode#1151), so nothing of the
  # door's behaviour is lost. bin/opencode-route-proxy.mjs, its
  # start script and test were removed in the same change.

  # Atlas search service (code#5350 step 5, #5355): the packaged
  # atlas-search binary's serve mode — one resident Snowflake
  # embedder shared by every client, the job protocol with its
  # in-memory temp store + dedupe cache, and per-client ceiling
  # caps enforced from ~/.local/state/atlas-search/service.json
  # (0600, written by `atlas-search init`). Binds 127.0.0.1:18760
  # (atlasSearchPort above); the tailnet exposure is the OpenCode
  # pattern — a `tailscale serve` mapping on the MagicDNS name in
  # front of the loopback port — and is created at activation
  # (step 6), not here: a serve mapping in front of a service
  # that has not started would be a live exposure change, and
  # tailscale serve state is imperative tailscaled state in this
  # fleet (no declarative precedent to land it in).
  #
  # Landed disabled in step 5 (no Install section, so the
  # unit could not auto-start); step 6 (#5356, the cutover)
  # added Install.WantedBy below, issued the real client
  # tokens via `atlas-search init`, and created the
  # tailscale serve mapping.
  systemd.user.services.atlas-search = {
    Unit = {
      Description = "Atlas search service (hybrid corpus search, serve mode)";
      After = [ "network-online.target" ];
      Wants = [ "network-online.target" ];
    };
    Service = {
      ExecStart = "${atlasSearch}/bin/atlas-search serve";
      Restart = "on-failure";
      RestartSec = 3;
    };
    # Install added at the step-6 cutover (#5356): auto-start.
    Install = {
      WantedBy = [ "default.target" ];
    };
  };

  # Fleet broker roles (epic #5412 step 2, #5430): the secrets and
  # queue brokers as `fleet serve <role>` units — one unit per role,
  # as they have run since code#5362. This is the home-module
  # pattern ported from dezren39/nix PR #1459 (closed as superseded
  # by ruling D1) into this tree, now consuming this flake's own
  # pkgs/fleet instead of a vendored copy. The unit files these
  # definitions replace were hand-installed; home-manager converges
  # them on the next activation, and the step-2 flips re-point the
  # live files at the same artifact in the meantime.
  systemd.user.services.secrets-broker = {
    Unit = {
      Description = "secrets-broker — resident Bitwarden secrets cache (code#5362)";
      After = [ "network-online.target" ];
    };
    Service = {
      ExecStart = "${fleetPkgs.fleet}/bin/fleet serve secrets-broker";
      Restart = "on-failure";
      RestartSec = 5;
    };
    Install.WantedBy = [ "default.target" ];
  };

  systemd.user.services.queue-broker = {
    Unit = {
      Description = "queue-broker — repo-health queue service (code#5362)";
      After = [
        "network-online.target"
        "secrets-broker.service"
      ];
      Wants = [ "secrets-broker.service" ];
    };
    Service = {
      ExecStart = "${fleetPkgs.fleet}/bin/fleet serve queue-broker";
      Restart = "on-failure";
      RestartSec = 5;
    };
    Install.WantedBy = [ "default.target" ];
  };

  systemd.user.services.sync-models = {
    Unit = {
      Description = "Sync AI models for OpenCode providers from models.dev and agy";
      After = [ "network-online.target" ];
    };
    Service = {
      Type = "oneshot";
      # #4544: sync-models maintains the runtime overlay, never the tracked
      # configured default — its rewrites must not dirty the repo tree.
      Environment = [
        "OPENCODE_CONFIG_PATH=%h/.local/state/opencode/opencode-runtime.jsonc"
      ];
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

  # ── Service cascade (2026-11-03) ─────────────────────────────────
  # The three standing services are defined by the fleet's layered
  # modules now, not by hand-rolled units in this file:
  #
  #   services.openchamber  developing-today-forks/openchamber, nix/modules
  #   services.opencode     developing-today-forks/opencode fork, nix/modules
  #   services.mcpx         developing-today/code, pkgs/mcpx/modules
  #
  # Each layer's enable cascades to the one below with mkDefault, so
  # the single switch here turns the whole stack on. OpenChamber
  # attaches to the standing OpenCode server (openchamber#766, the
  # one-process rule): the module's start wrapper reads the server's
  # registration file, exports OPENCODE_PASSWORD / OPENCODE_HOST /
  # OPENCODE_SKIP_START, reaps leftover servers on the port, and
  # refuses to start unattached. What remains in this file is only
  # this deployment's consumer configuration for the three modules,
  # plus the mcpx handover machinery the modules do not carry.
  # The cascade modules. NOTE: this file is merged into each user
  # with lib.merge (a recursiveUpdate fold — see home/user and
  # home/root), which keeps only one `imports` list; home/user
  # re-declares the effective union there. This line is what applies
  # for single-file merges (home/root).
  imports = [ inputs.openchamber.homeManagerModules.openchamber ];

  services.openchamber = {
    enable = true;
    # uiPasswordFile (the browser-UI password the old serve script
    # read from ~/.config/openchamber/ui-password) is a per-user
    # absolute path, so it is set in home/user, not in this shared
    # file. (The CLOUDFLARE_* exports the old script also made are
    # not lost with it: those values live in the systemd user
    # manager's environment, via the hand-maintained
    # ~/.config/environment.d/50-cloudflare.conf, and every user unit
    # inherits them — the mcpx daemon receives them the same way.)
  };

  services.opencode = {
    # enable comes from the cascade (mkDefault at each layer).
    #
    # package: the cascade build with the wrapper baked in
    # (opencodeWrapped, in the let above) -- one derivation for the
    # service unit and the home profile (Drew's ruling, 2026-11-03;
    # code#5038, option A). As a standalone home.packages entry the
    # wrapper collided with this package on bin/opencode, and it
    # exec'd the root input's build besides (CLI/service skew).
    package = opencodeWrapped;

    environment = {
      # code#5490: the runtime overlay (code#4544) must be layered into
      # the server process itself. As a home.sessionVariables entry it
      # reached shells only; the systemd unit never saw it, and the
      # standing server ran without the overlay after the 2026-11-04
      # reboot (overlay-only models, e.g. all of antigravity-direct,
      # unavailable) until a drop-in restored it live.
      OPENCODE_CONFIG = "%h/.local/state/opencode/opencode-runtime.jsonc";
      # opencode#1633: the fork agy client-secret extraction matches
      # the nix agy build binary layout, not the self-updated
      # ~/.gemini/bin build (1.4.11 moved the pairing secret). Pin the
      # intel source to the nix build until the fork iterates
      # candidates; remove then.
      AGY_BIN = "/run/current-system/sw/bin/agy";
    };
  };

  # DEEPSEEK_API_KEY for the opencode server: the value lives in
  # Bitwarden (deepseek.api-key), mirrored into sops and decrypted to
  # /run/secrets/agent-auth/deepseek-api-key (nixos/agent-auth). It must
  # never enter the Nix store, so an ExecStartPre materializes a
  # systemd EnvironmentFile at every server start — the same pattern
  # as openclaw-facade-auth-materialize in home/openclaw — and the
  # unit reads it via EnvironmentFile. It keys the deepseek provider's
  # official-API leg (deepseek/deepseek-flash, "1017") in the
  # deepseek-flash-plan route (Drew, direct order, 2026-11-05).
  #
  # MISTRAL_API_KEY is deliberately NOT materialized here (removed
  # 2026-11-06, root cause on opencode#1653). Mistral meters per KEY,
  # not by call metadata: the Bitwarden mistral.api-key value is the
  # account API key, billed to the API budget, while Vibe-plan usage
  # is billed only when the Vibe sign-in key is used (OS keyring
  # service "ai.mistral.vibe", from `vibe --setup`). vibe-direct
  # resolves its credential exactly like the vibe binary — env
  # MISTRAL_API_KEY first — so materializing the API key here
  # silently rerouted all vibe-direct traffic onto the API budget:
  # it burned the pool 16% -> 100% on 2026-11-05 and then walled the
  # mistral-plan route. With no MISTRAL_API_KEY in the server env,
  # vibe-direct falls through to the keyring plan key. The mistral
  # provider is unaffected: its enrolled credential (integration
  # "mistral") is its source — verified live 2026-11-06 (the leg
  # returns the API-budget 402, not an auth error, with the env var
  # absent).
  systemd.user.services.opencode.Service = {
    ExecStartPre = toString (
      pkgs.writeShellScript "opencode-env-key-materialize" ''
        set -eu
        umask 077
        mkdir -p "$HOME/.local/state/opencode"
        printf 'DEEPSEEK_API_KEY=%s\n' "$(cat /run/secrets/agent-auth/deepseek-api-key)" >> "$HOME/.local/state/opencode/mistral-api-key.env"
      ''
    );
    EnvironmentFile = "%h/.local/state/opencode/mistral-api-key.env";
  };

  services.mcpx = {
    # enable comes from the cascade (mkDefault at each layer).
    #
    # package: deliberately unset. There is one mcpx — the
    # cascade's (the fork's build, via the module wrapper's
    # mkDefault) — and this config takes that one (Drew's ruling,
    # 2026-11-03; code#5027). The former pin to this repo's own
    # pkgs/mcpx build made two mcpx builds coexist and collided
    # with the fork package's bin/mcpx in the home profile.
    #
    # addPackageToProfile: off. The profile's bin/mcpx comes from
    # exactly one provider — the opencode package's bundled
    # symlink. The module also adding cfg.package to home.packages
    # would put a second mcpx entry in the profile (Drew's ruling,
    # 2026-11-03; code#5027).
    addPackageToProfile = false;
    # The daemon's environment, unchanged from the hand-rolled unit
    # (mcpxEnvironment above).
    environment = mcpxEnvironment;
    # The daemon's configuration: mcpxSettings (mcpx-settings.nix,
    # the former ~/code/.mcpx.json). The module renders it to the
    # store, starts the daemon with --config pointing there, links
    # ~/.config/mcpx/config.json to the rendered file, and points
    # MCPX_CONFIG at it for other clients.
    settings = mcpxSettings;
  };

  # Consumer refinements on the module's mcpx unit: the parts of the
  # hand-rolled unit the module does not express. (Description,
  # ExecStart, Restart = always and the 20 s stop window are the
  # module's own.)
  systemd.user.services.mcpx = {
    Unit = {
      After = [ "network-online.target" ];
      Wants = [ "network-online.target" ];
      # A switch must not restart the daemon: that would end every
      # pooled MCP child. mcpxDeploy below hands a changed daemon
      # over instead.
      "X-RestartIfChanged" = false;
    };
    Service = {
      Type = "notify";
      NotifyAccess = "all";
      ExecReload = "${mcpx}/bin/mcpx reload";
      RestartSec = 3;
    };
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
  # --tailscale-serve-port 8443 is load-bearing (dezren39/nix#1279): without
  # it t3 configures Tailscale Serve on the default HTTPS port 443, which on
  # this host belongs to the PM stack aggregator (127.0.0.1:4870) -- every
  # t3code (re)start stole :443 back (hit twice on 2026-10-31). 8443 is
  # T3 Code's tailnet port; :443 must stay with the aggregator.
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
      ExecStart = "${latestCli.t3}/bin/t3 serve --tailscale-serve --tailscale-serve-port 8443 --no-browser";
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
    #   cloudflare-{account-id,gateway-id,ai-inference-token} | ~/.config/cloudflare/...
    #
    # The hook is sourced from bashrcExtra, which home-manager places BEFORE the
    # `[[ $- == *i* ]] || return` guard (initExtra comes after it, which is why
    # these exports used to be invisible to hidden agent shells). Non-interactive
    # `bash -c` shells get it through BASH_ENV instead (nixos/agent-auth +
    # systemd.user.sessionVariables below). See pkgs/agent-shell-env/env.sh.
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
      package = inputs.nixpkgs-unstable.legacyPackages.${system}.ghostty;
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
      package = firefoxBidi; # structural WebDriver BiDi on 127.0.0.1:19225 (code#5924)
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
      CODEX_PROXY_PORT = toString codexProxyPort;
      # MUSE_PROXY_PORT below was one of three missing session
      # variables whose {env:...} templates in opencode.jsonc
      # resolved to an empty port in the OpenChamber serve
      # (2026-10-25 audit) because the variables never reached its
      # environment; the other two (commandcode, claude-code) were
      # removed with their retired proxies on 2026-11-06. The session
      # list must cover every let-block port.
      MUSE_PROXY_PORT = toString museProxyPort;
      OPENCODE_EXPERIMENTAL_DISABLE_COPY_ON_SELECT = "1";
      # File watcher off by default (Drew, 2026-10-20): the working trees
      # are giant, and watching costs more than the live-external-edit
      # awareness it buys. Diffs are git-computed on demand either way.
      # (Name corrected 2026-10-25 per #698: opencode's server-process.ts
      # reads OPENCODE_DISABLE_FILEWATCHER; the EXPERIMENTAL_ name was
      # read nowhere, so the watcher was silently still on.)
      OPENCODE_DISABLE_FILEWATCHER = "1";
      # Config split (#4544): the runtime overlay — the provider catalogs
      # sync-models maintains, plus the server's own runtime config patches
      # — layers over the tracked configured default
      # (.opencode/opencode.jsonc). OPENCODE_CONFIG is the server's explicit
      # config layer; OPENCODE_CONFIG_PATH points sync-models at the file.
      OPENCODE_CONFIG = "$HOME/.local/state/opencode/opencode-runtime.jsonc";
      OPENCODE_CONFIG_PATH = "$HOME/.local/state/opencode/opencode-runtime.jsonc";
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
        # Atlas search CLI + service binary (pkgs/atlas-search);
        # step-6 cutover (#5356): the Rust binary is the resolved
        # `atlas-search` (the Python CLI is kept as atlas-search-py).
        atlasSearch
        # No "agy-proxy" / "antigravity-ls-proxy" launchers here: both
        # doors were retired 2026-10-30, their units removed above, and
        # their .mjs files removed in the 2026-11-06 proxy-infra
        # decommission — a PATH launcher would resurrect a retired door
        # on its canonical port.
        (pkgs.writeShellScriptBin "codex-proxy" ''
          export PATH="/run/current-system/sw/bin:$HOME/.local/bin:$PATH"
          exec ${pkgs.nodejs}/bin/node ${../../bin}/codex-proxy.mjs "$@"
        '')
        (pkgs.writeShellScriptBin "sync-models" ''
          export PATH="/run/current-system/sw/bin:$HOME/.gemini/bin:$HOME/.local/bin:$PATH"
          exec ${pkgs.nodejs}/bin/node ${../../bin/sync-models.mjs} "$@"
        '')
        (pkgs.writeShellScriptBin "update-models" ''
          export PATH="/run/current-system/sw/bin:$HOME/.gemini/bin:$HOME/.local/bin:$PATH"
          exec ${pkgs.nodejs}/bin/node ${../../bin/sync-models.mjs} "$@"
        '')
        # No standalone "opencode" script in this list: the CLI is
        # services.opencode.package -- the cascade build with the
        # wrapper baked in (opencodeWrapped, in the let above; code#5038).
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
        braveCdp # brave + structural CDP on 127.0.0.1:19223 (code#5924)
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
