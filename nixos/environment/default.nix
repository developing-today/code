{
  inputs,
  system,
  pkgs,
  lib,
  ...
}:
let
  # The compositor is the master flake build (see nixos/hyprland). Anything that
  # bakes hyprctl into its PATH must use the matching one, or it will ship the
  # nixpkgs 0.56.2 hyprctl and drag a second hyprland into the closure.
  hyprland-master = inputs.hyprland.packages.${system}.hyprland;
  espIdf5 = inputs.esp-dev.packages.${system}.esp-idf-full;
  espIdf6 = inputs.esp-dev-6.packages.${system}.esp-idf-full;
  mkEspIdfSystemPackage =
    {
      espIdf,
      version,
      default ? false,
    }:
    let
      toolEnv = lib.concatStringsSep "\n" (
        lib.mapAttrsToList (name: value: "export ${name}=${lib.escapeShellArg value}") espIdf.toolEnv
      );
      path = lib.makeBinPath (
        [
          pkgs.git
          pkgs.wget
          pkgs.gnumake
          pkgs.flex
          pkgs.bison
          pkgs.gperf
          pkgs.pkg-config
          pkgs.cmake
          pkgs.ninja
          pkgs.ncurses5
          pkgs.dfu-util
        ]
        ++ builtins.attrValues espIdf.tools
      );
      environment = ''
        export IDF_PATH=${lib.escapeShellArg "${espIdf}"}
        export IDF_TOOLS_PATH="$IDF_PATH/tools"
        export IDF_PYTHON_CHECK_CONSTRAINTS=no
        export IDF_PYTHON_ENV_PATH=${lib.escapeShellArg "${espIdf}/python-env"}
        export GIT_CONFIG_SYSTEM=${lib.escapeShellArg "${espIdf}/etc/gitconfig"}
        ${toolEnv}
        export PATH="$IDF_PYTHON_ENV_PATH/bin":${lib.escapeShellArg path}:"$IDF_PATH/tools":"$IDF_PATH/components/espcoredump":"$IDF_PATH/components/partition_table":"$IDF_PATH/components/app_update":$PATH
      '';
      idfCommand = pkgs.writeShellScriptBin "idf${version}.py" ''
        ${environment}
        exec "$IDF_PYTHON_ENV_PATH/bin/python" "$IDF_PATH/tools/idf.py" "$@"
      '';
      idfShell = pkgs.writeShellScriptBin "esp-idf-${version}" ''
        ${environment}
        exec ${pkgs.bashInteractive}/bin/bash "$@"
      '';
      defaultCommand = pkgs.writeShellScriptBin "idf.py" ''
        exec ${idfCommand}/bin/idf${version}.py "$@"
      '';
    in
    pkgs.symlinkJoin {
      name = "esp-idf-${version}-system";
      paths = [
        espIdf
        idfCommand
        idfShell
      ]
      ++ lib.optional default defaultCommand;
    };
  espIdf5System = mkEspIdfSystemPackage {
    espIdf = espIdf5;
    version = "5";
    default = true;
  };
  espIdf6System = mkEspIdfSystemPackage {
    espIdf = espIdf6;
    version = "6";
  };
  # SquareLine Studio: proprietary LVGL UI editor, distributed as an unpackaged zip.
  # Update version/url/hash on new releases from https://squareline.io/downloads
  squareline-studio =
    let
      squareline-studio-unwrapped = pkgs.stdenv.mkDerivation {
        pname = "squareline-studio-unwrapped";
        version = "1.6.1";
        src = pkgs.fetchurl {
          url = "https://static.squareline.io/downloads/SquareLine_Studio_Linux_v1_6_1.zip";
          hash = "sha256-KLz71HWtFnDsaIEXy/7rvWsL7bUrFuZAEdTG7spHq10=";
        };
        nativeBuildInputs = [ pkgs.unzip ];
        sourceRoot = ".";
        dontPatchELF = true;
        dontAutoPatchelf = true;
        installPhase = ''
          mkdir -p $out/lib/squareline-studio
          cp -r . $out/lib/squareline-studio/
          find $out/lib/squareline-studio -type f \( -name "*.x86_64" -o -name "*.so" \) -exec chmod +x {} +
        '';
      };
      squareline-run =
        (pkgs.writeShellScriptBin "squareline-run" ''
          exe=$(find ${squareline-studio-unwrapped}/lib/squareline-studio -type f -name SquareLine_Studio.x86_64 | head -n1)
          exec "$exe" "$@"
        '').overrideAttrs
          (_: {
            passthru.squareline-studio-unwrapped = squareline-studio-unwrapped;
          });
    in
    pkgs.buildFHSEnv {
      name = "squareline-studio";
      targetPkgs =
        p: with p; [
          alsa-lib
          curl
          dbus
          fontconfig
          freetype
          gtk3
          libGL
          nspr
          nss
          udev
          xorg.libX11
          xorg.libXcursor
          xorg.libXext
          xorg.libXi
          xorg.libXrandr
          xorg.libXrender
          zlib
        ];
      runScript = "${squareline-run}/bin/squareline-run";
      extraInstallCommands = ''
        mkdir -p $out/share/applications
        sed "s|__folder__|squareline-studio|g" ${squareline-studio-unwrapped}/lib/squareline-studio/squareline_studio.desktop.template > $out/share/applications/squareline-studio.desktop || true
      '';
    };
  my-kubernetes-helm = pkgs.wrapHelm pkgs.kubernetes-helm {
    plugins = builtins.attrValues (
      lib.filterAttrs (name: _: lib.hasPrefix "helm-" name) pkgs.kubernetes-helmPlugins
    );
  };
  my-helmfile = pkgs.helmfile-wrapped.override { inherit (my-kubernetes-helm) pluginsDir; };

  # opencode-desktop: upstream rewrote the desktop app (electron/bun, no more tauri/cargo),
  # so the old outputHashes overrideAttrs is no longer needed
  opencode-desktop = inputs.opencode.packages.${system}.opencode-desktop;

  # OpenCode packages:
  # opencode-2x is the default on PATH as `opencode` (and `opencode2`).
  # opencode v1 (1.18.x) is retained under `opencode-v1`, `opencode1`, and `opencode-1x`.
  opencode-v1 =
    pkgs.runCommand "opencode-v1"
      {
        meta = (inputs.opencode.packages.${system}.opencode.meta or { }) // {
          mainProgram = "opencode-v1";
        };
      }
      ''
        mkdir -p $out/bin
        ln -s ${inputs.opencode.packages.${system}.opencode}/bin/opencode $out/bin/opencode-v1
        ln -s ${inputs.opencode.packages.${system}.opencode}/bin/opencode $out/bin/opencode1
        ln -s ${inputs.opencode.packages.${system}.opencode}/bin/opencode $out/bin/opencode-1x
      '';

  opencode-v2-compat = pkgs.runCommand "opencode-v2-compat" { } ''
    mkdir -p $out/bin
    ln -s ${inputs.opencode-2x.packages.${system}.opencode}/bin/opencode $out/bin/opencode-v2
  '';

  # 2x scaling for Electron apps.
  #
  # The GDK_SCALE / QT_SCALE_FACTOR pair set in environment.sessionVariables
  # does nothing for Electron: with NIXOS_OZONE_WL=1 these run as native Wayland
  # clients and take their scale from the compositor, and we are deliberately
  # NOT setting a Hyprland monitor scale (the whole point is to leave the bar and
  # lockscreen alone). Chromium exposes no environment variable for this -- only
  # the --force-device-scale-factor command-line flag -- so each app is wrapped.
  #
  # Every executable in bin/ is wrapped rather than a named one, so this does not
  # depend on each upstream's choice of binary name. Desktop entries are also
  # rewritten: several of these packages hard-code their own store path in Exec=,
  # which would launch the unwrapped binary straight from the store and silently
  # bypass the flag when started from a launcher rather than a shell.
  scaleElectron2x =
    pkg:
    pkgs.symlinkJoin {
      name = "${pkg.pname or pkg.name or "electron-app"}-scaled2x";
      paths = [ pkg ];
      nativeBuildInputs = [ pkgs.makeWrapper ];
      postBuild = ''
        for bin in "$out"/bin/*; do
          [ -L "$bin" ] || continue
          target=$(readlink -f "$bin")
          [ -x "$target" ] || continue
          rm "$bin"
          makeWrapper "$target" "$bin" \
            --add-flags "--force-device-scale-factor=2"
        done

        if [ -d "$out/share/applications" ]; then
          for f in "$out"/share/applications/*.desktop; do
            [ -e "$f" ] || continue
            src=$(readlink -f "$f")
            base=$(basename "$f")
            rm "$f"
            sed "s|${pkg}/bin/|$out/bin/|g" "$src" > "$out/share/applications/$base"
          done
        fi
      '';
      meta = (pkg.meta or { }) // {
        mainProgram = pkg.meta.mainProgram or null;
      };
    };

  # OpenChamber's Electron GUI. Two deviations from the upstream flake package:
  #
  # 1. buildPhase: upstream never builds @openchamber/sdk. That package's
  #    package.json maps exports["./schemas"] -> ./dist/schemas.js, generated by
  #    its `build` script (tsc -p tsconfig.build.json). The flake passes
  #    --ignore-scripts, so no prepare hook runs it, and its buildPhase only
  #    builds packages/electron. The result ships sdk/src with no sdk/dist, and
  #    the app dies at startup with ERR_MODULE_NOT_FOUND on dist/schemas.js
  #    (the Electron shell boots an embedded web server that imports it).
  #    The published npm tarball used by the plain `openchamber` attr ships a
  #    prebuilt dist, so only this from-source desktop build is affected.
  #
  # 2. opencode: the flake injects its own nixpkgs opencode (1.18.25), but
  #    OpenChamber 2.1.0 requires >= 2.0.20 and otherwise logs
  #    "Continuing without OpenCode integration". Point it at the 2.x pin.
  openchamber-desktop =
    (inputs.openchamber.packages.${system}.openchamber-desktop.override {
      opencode = inputs.opencode-2x.packages.${system}.opencode;
    }).overrideAttrs
      (old: {
        buildPhase = ''
          runHook preBuild
          export HOME=$TMPDIR

          patchShebangs node_modules packages

          bun run --cwd packages/sdk build

          bun run --cwd packages/electron build:web-assets
          bun run --cwd packages/electron bundle:main
          ln -s ../resources packages/electron/dist-bundle/resources
          runHook postBuild
        '';
      });

  openchamber = inputs.openchamber.packages.${system}.openchamber.override {
    opencode = inputs.opencode-2x.packages.${system}.opencode;
  };

  # Meta's Muse Code agent. Hand-rolled because it is not in nixpkgs and the
  # only documented install is a `curl | sh` that self-updates; see the
  # derivation for how the artifact URL + upstream sha256 are pinned.
  # callPackage'd here rather than via pkgs/default.nix, which the NixOS
  # config does not import.
  muse-code = pkgs.callPackage (lib.from-root "pkgs/muse-code") { };

  # Vercel CLI: absent from nixpkgs (`vercel-pkg` there is the unrelated legacy
  # bundler) and npm-only -- upstream attaches no binaries to its releases.
  vercel-cli = pkgs.callPackage (lib.from-root "pkgs/vercel-cli") { };

  # Command Code agent CLI. UNFREE and license-less upstream (npm says
  # "UNLICENSED", repo has no SPDX file); see the derivation header before
  # redistributing or caching this anywhere shared.
  command-code = pkgs.callPackage (lib.from-root "pkgs/command-code") { };

  # Jules Tools, the CLI for Google's async coding agent. Not in nixpkgs; the
  # npm package is a stub that downloads a dynamically linked Go binary, which
  # will not exec on NixOS without autoPatchelfHook. See the derivation header.
  jules = pkgs.callPackage (lib.from-root "pkgs/jules") { };

  # Antigravity ACP server and Hub. Both vendored from nixpkgs master rather
  # than taken from a channel: neither exists in ANY nixpkgs revision this flake
  # pins (stable, unstable or master-as-pinned), and bumping an input to reach
  # two leaf packages would rebuild most of the system for no other gain. Every
  # dependency they need is already present in the pinned channel.
  #
  # antigravity-acp provides `agy_acp_server`, the official Agent Client
  # Protocol server. This is the one that matters for t3code: t3 ships an
  # AcpRegistryDriver, so ACP is the generic path for t3 to drive Antigravity.
  antigravity-acp = pkgs.callPackage (lib.from-root "pkgs/antigravity-acp") { };

  # "Antigravity Cockpit": account manager covering Antigravity, Codex, Copilot,
  # Cursor, Gemini CLI and others. 18.6k stars, no declared licence upstream.
  cockpit-tools = pkgs.callPackage (lib.from-root "pkgs/cockpit-tools") { };

  # Google's experimental Jules orchestration pair. Fleet drives goal files ->
  # analyzer sessions -> labelled issues -> worker sessions; Merge reconciles
  # the overlapping PRs that result. Google's own words on Fleet: "very
  # experimental, just for fun".
  # Grok CLI (superagent-ai/grok-cli, published to npm as `grok-dev`).
  # Provides `grok`, which t3code's `grok` driver shells out to.
  grok-cli = pkgs.callPackage (lib.from-root "pkgs/grok-cli") { };

  # Claude Code 2.1.289 (nixpkgs has 2.1.234 in both channel and unstable).
  claude-code = pkgs.callPackage (lib.from-root "pkgs/claude-code") { };

  jules-fleet = pkgs.callPackage (lib.from-root "pkgs/jules-fleet") { };
  jules-merge = pkgs.callPackage (lib.from-root "pkgs/jules-merge") { };
  antigravity-hub = pkgs.callPackage (lib.from-root "pkgs/antigravity-hub") { };

  # Upstream prebuilt release binaries for tools where nixpkgs trails upstream.
  # See pkgs/latest-cli/default.nix for rationale and tradeoffs.
  latestCli = pkgs.callPackage (lib.from-root "pkgs/latest-cli") { };
in
{
  nixpkgs.overlays = [
    # zulip-term: 4 tests fail on nixpkgs master 2026-08-21 (upstream test breakage)
    (final: prev: {
      zulip-term = prev.zulip-term.overridePythonAttrs (_: {
        doCheck = false;
      });
    })
    # neovim nightly: treesitter functional tests fail (nightly flakiness)
    (final: prev: {
      neovim-unwrapped = prev.neovim-unwrapped.overrideAttrs (_: {
        doCheck = false;
      });
    })
    # mise: 29 unit tests fail in sandbox (network-dependent tests)
    (final: prev: {
      mise = prev.mise.overrideAttrs (_: {
        doCheck = false;
      });
    })
  ];
  environment = {
    sessionVariables = {
      NIXOS_OZONE_WL = "1"; # This variable fixes electron apps in wayland
      NIXPKGS_ALLOW_UNFREE = "1";

      # 2x UI scaling for applications only -- Hyprland's own monitor scale is
      # deliberately left unset, so the compositor, bar and lockscreen are
      # unaffected and only app toolkits render larger.
      #
      # GTK: GDK_SCALE is an integer multiplier on the whole UI, fonts included.
      GDK_SCALE = "2";
      # Qt: QT_SCALE_FACTOR is the Qt equivalent. QT_AUTO_SCREEN_SCALE_FACTOR is
      # explicitly disabled so Qt does not additionally apply its own per-screen
      # DPI guess on top of the factor below and end up at 4x.
      QT_SCALE_FACTOR = "2";
      QT_AUTO_SCREEN_SCALE_FACTOR = "0";
      # Cursors are not scaled by either of the above, so size them to match
      # (the usual default is 24).
      XCURSOR_SIZE = "48";
      # NOTE: XWayland/X11-only clients honour neither GDK_SCALE nor
      # QT_SCALE_FACTOR reliably; they read the Xft.dpi X resource, which is set
      # via xrdb rather than the environment. If any X11 app stays small, that is
      # why. Electron is a separate case again -- see below.

      XDG_CACHE_HOME = "$HOME/.cache";
      # XDG_CONFIG_DIRS = "/etc/xdg";
      XDG_CONFIG_HOME = "$HOME/.config";
      # XDG_DATA_DIRS = "/usr/local/share/:/usr/share/";
      XDG_DATA_HOME = "$HOME/.local/share";
      XDG_STATE_HOME = "$HOME/.local/state";
    };
    variables = {
      EDITOR = "nvim";
      NIX_REMOTE = "daemon";
      PLAYWRIGHT_BROWSERS_PATH = "${inputs.nixpkgs-master.legacyPackages.${system}.playwright-driver.browsers
      }";
    };
    # things should end up in systempackages if
    # they are required for boot or login or
    # have namespace conflicts i don't want to deal with in home manager
    # or just because
    # etc.
    systemPackages = [
      espIdf5System
      espIdf6System
      my-helmfile
      my-kubernetes-helm
      (scaleElectron2x opencode-desktop)
      (scaleElectron2x openchamber-desktop)
      muse-code
      vercel-cli
      command-code
      jules
      antigravity-acp
      antigravity-hub
      claude-code
      cockpit-tools
      grok-cli
      jules-fleet
      jules-merge
      latestCli.codex
      opencode-v1
      opencode-v2-compat
      openchamber
    ]
    ++ (with inputs; [
      #rose-pine-hyprcursor.packages.${pkgs.system}.default
      nix-output-monitor.packages.${system}.default
      ssh-to-age.packages.${system}.default
      nix-search.packages.${system}.default
      zen-browser.packages.${system}.default
      #hyprland-qtutils.packages.${system}.hyprland-qtutils
      clan-core.packages.${system}.clan-cli
      opencode-2x.packages.${system}.opencode
    ])
    ++ [
      # Wrapped for 2x scaling; see scaleElectron2x above. These sit outside the
      # `with inputs` block because the wrapper is defined in the let binding,
      # not on inputs.
      #
      # claude-desktop: the -fhs output, not the plain one -- Claude Desktop's
      # MCP servers are npx/uvx/docker invocations that need a conventional
      # filesystem layout and break against a pure store path.
      (scaleElectron2x inputs.claude-desktop.packages.${system}.claude-desktop-fhs)
      (scaleElectron2x inputs.helium.packages.${system}.helium)
      # chatgpt: the -remote-mobile-control variant rather than plain
      # `codex-desktop`. OpenAI's own Codex Remote requires the host to run the
      # macOS or Windows desktop app ("you can't set it up from the Codex CLI or
      # IDE extension"), so stock Linux has no phone->this-machine path at all.
      # This build adds one. Swap to `codex-desktop` to run the unpatched
      # official payload instead.
      (scaleElectron2x inputs.chatgpt-desktop.packages.${system}.codex-desktop-remote-mobile-control)
    ]
    ++ (with inputs.roc.packages.${system}; [ nightly ])
    ++ (with inputs.affinity-nix.packages.${system}; [
      photo
      publisher
      designer
    ])
    # TODO: revert to nixpkgs, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
    ++ (with pkgs; [
      age
      wpa_supplicant_gui
      # Cloudflare Workers CLI; available as a native nixpkgs package.
      wrangler
    ])
    ++ (with pkgs; [
      # Embedded development: ESP32/ESP8266, Arduino, RP2040, AVR, ARM and RISC-V
      esptool
      esptool-ck
      espflash
      espflash # was cargo-espflash, renamed upstream
      cargo-espmonitor
      espup
      esp-generate
      python3Packages.esp-idf-size
      platformio
      gcc-arm-embedded
      pkgsCross.arm-embedded.stdenv.cc
      pkgsCross.arm-embedded.buildPackages.gdb
      pkgsCross.riscv32-embedded.stdenv.cc
      pkgsCross.riscv32-embedded.buildPackages.gdb
      pkgsCross.riscv64-embedded.stdenv.cc
      pkgsCross.riscv64-embedded.buildPackages.gdb
      pkgsCross.avr.stdenv.cc
      pkgsCross.avr.buildPackages.gdb
      pkgsCross.avr.libc # was bare avrlibc; top-level avrlibc now refuses to eval on x86_64

      # PDF mining for the hardware-doc knowledge base. Datasheets and reference
      # manuals arrive as PDF and have to be turned into cited Markdown; the
      # archiving rule in .agents/skills/hardware-device-research requires content
      # be mined before an artifact may be moved out of the repository.
      poppler-utils # pdftotext -layout, pdfinfo, pdfimages — the workhorse
      mupdf # mutool: structure, embedded files, page extraction where poppler chokes
      qpdf # decrypt/linearise/repair before extraction; inspect object structure
      avra
      avrdude
      simavr # upstream now uses pkgsCross.avr.libc internally
      gdb
      openocd
      openocd-rp2040
      probe-rs-tools
      pyocd
      stlink
      dfu-util
      dfu-programmer
      flashrom
      flashprog
      picotool
      pico-sdk
      elf2uf2-rs
      bossa-arduino
      teensy-loader-cli
      srecord

      # MicroPython and CircuitPython workflows
      (micropython.overrideAttrs (_: {
        # 10 tests fail on nixpkgs master 2026-08-21 (upstream breakage)
        doCheck = false;
      }))
      mpremote
      thonny
      rshell
      adafruit-ampy
      circup

      # Meshtastic, MeshCore, Reticulum/RNode and LoRaWAN
      meshtastic
      meshtasticd
      meshtastic-web
      meshcore-cli
      rns
      rnsapi
      rns-proxy
      rs-reticulum
      lxmf-rs
      reticulum-go
      reticulum-group-chat
      nomadnet
      sideband
      loramon
      chirpstack-gateway-bridge
      chirpstack-concentratord
      chirpstack-gateway-mesh
      chirpstack-mqtt-forwarder
      chirpstack-udp-forwarder
      chirpstack-rest-api
      chirpstack-fuota-server

      # Flipper Zero, Raspberry Pi and common board utilities
      qFlipper
      python3Packages.pyflipper
      rpi-imager
      raspberrypi-eeprom
      ubootTools
      binwalk
      cutter

      # Serial, USB, GPIO and hardware buses
      tio
      picocom
      serial-studio
      moserial
      gtkterm
      cutecom
      grabserial
      libserialport
      usbtree
      i2c-tools
      spi-tools
      can-utils
      savvycan

      # Logic analyzers, oscilloscopes and packet analysis
      sigrok-cli
      pulseview
      sigrok-firmware-fx2lafw
      wireshark
      kismet
      direwolf
      chirp
      mosquitto
      mqttx
      mqttx-cli
      mqtt-explorer
      mqttui
      home-assistant-cli
      python3Packages.paho-mqtt
      libcoap
      rtl_433

      # D-Bus/GObject Python bindings.
      # Mirrors the devshell python3.withPackages set in nix-common.nix so the
      # same scripts run outside `nix develop`.
      python3Packages.pydbus
      python3Packages.dbus-python
      python3Packages.pygobject3
      gobject-introspection
      glib

      # Wireless ecosystem tooling: Matter/Thread/Zigbee/BLE and embedded serialization
      esphome
      zigbee2mqtt
      bluez
      nanopb
      flatbuffers
      cbor-diag

      # Software-defined radio and LoRa signal analysis
      gnuradio
      gnuradioPackages.osmosdr
      gnuradioPackages.lora_sdr
      gqrx
      sdrangel
      urh
      inspectrum
      hackrf
      rtl-sdr
      soapysdr-with-plugins
      uhd
      airspy
      airspyhf
      libbladeRF

      # FPGA, HDL and programmable logic
      yosys
      nextpnrWithGui
      nextpnr-xilinx
      iverilog
      verilator
      ghdl
      gtkwave
      sby
      icestorm
      trellis
      openfpgaloader
      icesprog
      fujprog
      vhdl-ls
      verible
      slang
      surfer

      # Electronics, displays and firmware asset creation
      kicad
      fritzing
      librepcb
      horizon-eda
      # geda # removed from nixpkgs 2026-07-26: unmaintained upstream
      gerbv

      # Mechanical CAD / STEP + FreeCAD inspection
      # Used for vendor mechanical design files during hardware research, e.g.
      # doc/hardware/devices/nicolai-electronics/tanmatsu (case .FCStd + .step).
      # freecad ships `freecadcmd` for headless scripting.
      # NOTE: CadQuery is NOT in nixpkgs (no `cadquery`, no `OCP` attribute).
      # pythonocc-core gives equivalent OpenCascade bindings; for CadQuery
      # proper use a venv:  uv venv && uv pip install cadquery
      freecad
      python3Packages.pythonocc-core
      python3Packages.ezdxf

      appimage-run # for other proprietary AppImage tools
      squareline-studio
      imagemagick
      lv_font_conv
      pngquant
      optipng
      oxipng
      svgo
      resvg
      potrace

      # GPS/GNSS receivers, mapping and positioning
      gpsd
      gpsbabel
      gpsprune
      gpxsee
      # foxtrotgps # removed from nixpkgs: GTK2/libglade deprecated
      gnss-sdr
      gnss-share
      rtklib-ex
    ])
    ++ (with inputs.nixpkgs-25.legacyPackages.${system}; [ activitywatch ])
    ++ (with inputs.nixpkgs-stable.legacyPackages.${system}; [ ])
    ++ (with inputs.nixpkgs-unstable.legacyPackages.${system}; [
      # AI coding agents. Kept on the unstable channel rather than master
      # because unstable is cached (see the chromium note below) and these
      # are large node/electron closures.
      # claude-code moved out of this list: nixpkgs (channel and unstable both)
      # carries 2.1.234 while npm is on 2.1.289. Now from pkgs/claude-code,
      # which pins the platform-native binary directly -- see that file for why
      # the npm wrapper package cannot be used.
      # codex moved out of this list: nixpkgs-unstable carries 0.147.0 while
      # upstream is at rust-v0.160.0 (13 minor versions). Now taken from
      # pkgs/latest-cli as an upstream prebuilt musl-static release binary --
      # see the `++ [ ... ]` block below.
      antigravity-ide # google agentic IDE (unfree). `antigravity` is an alias.
      antigravity-cli # google, mainProgram "antigravity"
      # gemini-cli: nixpkgs carries a removal notice ("Unpaid tier and Google AI
      # Pro/Ultra users: Gemini CLI was replaced by Antigravity CLI"), which is
      # an eval *warning*, not a build failure -- 0.47.0 builds and runs fine.
      # Installed deliberately: upstream google-gemini/gemini-cli is very much
      # alive, and the `gemini` binary is required by the official
      # gemini-cli-extensions/jules extension.
      gemini-cli
    ])
    ++ (with inputs.nixpkgs-master.legacyPackages.${system}; [
      ghostty
      zed-editor
      # opencode
    ])
    ++ (with inputs.nixpkgs-master.legacyPackages.${system}; [
      # rclone   # fish completions broke 2025-04-03
      # rclone-browser   # fish completions broke 2025-04-03
      # rclone-ui   # fish completions broke 2025-04-03
      framework-tool
      musl
      fish
      tcl
      nushell
      ripgrep
      fd
      bat
      eza
      zoxide
      xh
      zellij
      gitui
      dust
      dua
      starship
      yazi
      hyperfine
      evil-helix
      bacon
      cargo-info
      fselect
      ncspot
      rusty-man
      delta
      ripgrep-all
      tokei
      wiki-tui
      just
      mask
      mprocs
      presenterm
      kondo
      # bob-nvim
      bun
      nodejs # npm, npx
      (mise.overrideAttrs (old: {
        # 29 unit tests fail in sandbox (network-dependent); raw master input bypasses nixpkgs.overlays
        doCheck = false;
        # libz-ng-sys build script requires cmake
        nativeBuildInputs = (old.nativeBuildInputs or [ ]) ++ [ pkgs.cmake ];
      })) # rtx
      espanso

      # TODO: cleanup systemPackages
      # build
      # charm stuff?
      # dwm
      # fortune
      # gtk
      # inputs.omnix.packages.${system}.default
      # omnix
      # overlays # todo- move into user
      # clang-tools_9
      # fontmatrix
      # grep
      # nix-software-center
      # zed-editor
      # zigpkgs.master
      unison-ucm
      brotli
      # unison-fsmonitor
      simple-http-server
      arduino
      arduino-cli
      arduino-core
      #arduino-create-agent # fish-completions as usual
      arduino-ide
      arduino-language-server
      arduino-mk
      arduino-ota # renamed from arduinoOTA
      #code-cursor
      gamemode
      argo-workflows
      argocd
      argocd-autopilot
      solaar
      gnomeExtensions.solaar-extension
      logitech-udev-rules
      horst
      smartmontools
      nvme-cli
      kubectl
      kubectl-tree
      kubectl-ktop
      kubectl-df-pv
      kubectl-neat
      kubectl-doctor
      kubectl-explore
      kubectl-example
      kubectl-view-allocations
      kubectl-view-secret
      kubectl-graph
      kubectl-gadget
      kubectl-images
      kubectl-node-shell
      helm
      helm-ls
      k6
      krew
      # helmfile
      # kubernetes-helm-wrapped
      # helmfile-wrapped
      helmsman
      helmsman
      helm-docs
      helm-dashboard
      helm-docs
      kustomize-sops
      kustomize
      kubernetes-code-generator
      kubernetes-controller-tools
      # kubernetes-helm-wrapped
      # kubernetes-helmPlugins
      kubernetes-kcp
      kubernetes-metrics-server
      kubernetes-polaris
      kubernetes
      kubecolor
      k3sup
      k3s
      k3d
      prometheus
      prometheus-alertmanager
      #grafana
      #grafana-loki
      #grafana-image-renderer
      #grafana-reporter
      #grafana-alloy
      #grafana-agent
      opentelemetry-collector
      tempo
      temporal
      mimir
      wavemon
      nordzy-icon-theme
      # nordzy-cursor-theme
      # fdd # TODO
      # wpe # TODO
      # we # TODO
      httping
      # rtv # TODO
      # scrap # TODO
      socat
      lshw
      qemu
      space-cadet-pinball
      alacritty-theme
      alejandra # unused now?
      asciinema
      awesome
      banner
      bc
      binutils
      brillo
      bsdgames
      cabal-install
      cabal2nix
      choose
      cinnamon-desktop
      nixd
      nil
      guake
      # python3-dbus
      uv
      python312Packages.pydbus
      python312Packages.pygobject3
      clang
      cowsay
      talosctl
      e2fsprogs
      emacs.pkgs.fortune-cookie

      # fancycat
      libx11
      # xorg.libXcursor
      libxi
      libxinerama
      libxrandr
      alsa-lib
      # emscripten
      # libGL
      # libsixel
      # libxkbcommon
      # lsix
      # mesa.drivers
      cargo
      rustc
      rustup
      # simple-http-server
      timg
      tiny
      tmux
      wayland
      zig # For Web support, used to build roc wasm static library
      expect # unbuffer
      figlet
      fira-code
      fira-code-symbols
      font-awesome
      font-awesome_5
      font-manager
      fontforge
      fontpreview
      fortune
      # jmtpfs # removed from nixpkgs: unmaintained (simple-mtpfs below)
      go-mtpfs
      usbutils # for lsusb
      libmtp
      simple-mtpfs # or jmtpfs, go-mtpfs, etc.
      android-file-transfer # GUI/CLI MTP client
      android-tools # adb, fastboot
      gawk
      gcc
      gdm
      ghc
      github-copilot-cli
      # gnomeExtensions.toggle-alacritty # TODO: broke with 26
      # grimblast is a wrapped shell script; it bakes hyprctl into its PATH.
      # Default would be pkgs.hyprland (0.56.2). Its queries (clients,
      # activewindow) are still valid IPC commands on master.
      (grimblast.override { hyprland = hyprland-master; })
      gtk2
      gtk3
      gtk4
      #hackgen-nf-font
      haskellPackages.misfortune
      hasklig
      hledger
      hledger-iadd
      hledger-interest
      usbutils
      usbtop
      usbrip
      usbview
      usbimager
      ns-usbloader
      woeusb
      gparted
      woeusb-ng

      hledger-ui
      hledger-utils
      # zen-browser
      hledger-web
      # hyprcursor
      hyprdim
      hyprkeys
      hyprland-monitor-attached
      hyprland-protocols
      #inputs.hypr-dynamic-cursors.packages.${pkgs.system}.hypr-dynamic-cursors
      hyprlock
      hyprpicker
      # hyprshade 5.0.0 already migrated to the Lua config (upstream #69): it
      # sets shaders via `hyprctl eval 'hl.config({decoration={screen_shader=...}})'`
      # and reads them via `hyprctl -j getoption`. Both are valid master IPC
      # commands, so it needs the master hyprctl, not nixpkgs 0.56.2's.
      (hyprshade.override { hyprland = hyprland-master; })
      # hyprshot only queries (activewindow/monitors/clients), all still valid.
      (hyprshot.override { hyprland = hyprland-master; })
      kanata
      kitti3
      kitty
      kitty-img
      kitty-themes
      kittysay
      lf
      libsixel
      libusb1
      libusb-compat-0_1
      pkg-config
      libusb1
      hidapi
      lightdm
      llvmPackages.bintools
      lolcat
      lsix
      #maple-mono.NF
      ##maple-mono.SC-NF
      #maple-mono.Normal-OTF
      #maple-mono.Normal-TTF-AutoHint
      #maple-mono.Normal-TTF
      #maple-mono.Normal-Woff2
      #maple-mono.Normal-NF
      #maple-mono.Normal-Variable
      #maple-mono.variable
      minicom
      monoid
      ncdu
      ncurses
      neovim
      nerd-font-patcher
      nerdfix
      nerdfix
      #nerdfonts
      nh
      niv
      nix-du
      nix-melt
      nix-output-monitor
      nix-query-tree-viewer
      nix-tree
      nix-visualize
      # Formatters and linters
      nixfmt
      treefmt
      statix
      deadnix
      shfmt
      shellcheck
      prettier # nodepackages remoed 2026-04-03
      ruff
      biome
      rustfmt
      taplo
      rufo
      elmPackages.elm-format
      go
      haskellPackages.ormolu
      nushell
      nvd
      oils-for-unix # todo: osh default shell?
      opentofu
      pixcat
      playerctl
      python312Packages.pycritty
      rPackages.fortunes
      ranger
      rictydiminished-with-firacode
      rescuetime
      ddrescue
      magicrescue
      ddrutility
      myrescue
      ddrescueview
      unetbootin # can't launch right now? qt platform platform plugin not found
      # dd_rescue
      #ventoy-full # https://www.ventoy.net/en/doc_search_path.html
      #  Known issues:
      #        - Ventoy uses binary blobs which can't be trusted to be free of malware or compliant to their licenses.
      #       https://github.com/NixOS/nixpkgs/issues/404663
      #       See the following Issues for context:
      #       https://github.com/ventoy/Ventoy/issues/2795
      #       https://github.com/ventoy/Ventoy/issues/3224
      # ventoy
      screen
      #sddm
      netboot
      ipxe
      # waitron
      # https://theartofmachinery.com/2016/04/21/partitioned_live_usb.html
      # https://www.system-rescue.org/
      # https://discourse.nixos.org/t/how-to-add-a-rescue-option-to-bootloader/19137
      # specialisation rescue disk
      # specialisation live disk
      # specialisation usb live disk
      # https://nixos.wiki/wiki/Change_root
      # https://nixos.wiki/wiki/Bootloader#From_an_installation_media
      # https://wiki.gentoo.org/wiki/LiveUSB#Linux
      pixiecore
      # yumi # no package yet :(
      # netbootxyz-efi # WARNING: caused failed rebuild
      # netbootxyz
      # tinkerbell
      # matchbox-server
      # terraform-providers.<provider>
      # https://github.com/DeterminateSystems/nix-netboot-serve
      ubootTools
      # uboot<raspberryModel>
      statix
      syslinux
      tailscale
      taoup
      #terminus-nerdfont
      # termpdfpy # 2024-09-17 ⚠ python3.12-pymupdf-1.24.8 failed with exit code 1 after ⏱ 1m55s in pythonImportsCheckPhase
      terranix
      udev-gothic-nf
      vimPlugins.vim-kitty-navigator
      waybar
      wayland
      # xdg-desktop-portal-hyprland
      # xorg.xcursorthemes
      xwayland
      yazi
      yq
      zathura
      zathura
      magic-wormhole-rs
      wormhole-william
      magic-wormhole
      webwormhole
      portal
      cdrkit
      cdrtools
      # age # TODO: move to nixpkgs-unstable, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
      libisoburn # xorriso
      # wpa_supplicant_gui # TODO: move to nixpkgs-unstable, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
      # wpa_cute # TODO: try this?
      element-web
      element-call
      element-desktop

      # Security and authentication
      _1password-gui
      yubikey-agent
      keepassxc

      # App and package management
      appimage-run
      gnumake
      cmake
      home-manager

      # Media and design tools
      ffmpeg
      gimp
      vlc
      wineWow64Packages.stable # renamed from wineWowPackages
      #fontconfig
      font-manager

      # Printers and drivers
      brlaser # printer driver

      # Calculators
      bc # old school calculator
      # galculator # broke with 26, maybe try again later?

      # Audio tools
      cava # Terminal audio visualizer
      pavucontrol # Pulse audio controls

      # Messaging and chat applications
      # cider # Apple Music on Linux; removed from nixpkgs: unmaintained, archived upstream
      discord
      # hexchat # removed from nixpkgs: archived upstream, gtk2
      fractal # Matrix.org messaging app
      #tdesktop # telegram desktop

      # Testing and development tools
      #beekeeper-studio # electron 31 eol
      cypress # Functional testing framework using headless chrome
      inputs.nixpkgs-unstable.legacyPackages.${system}.chromium # nixos-unstable channel: cached (master chromium is not)
      inputs.nixpkgs-unstable.legacyPackages.${system}.chromedriver
      playwright-driver
      direnv
      rofi
      rofi-calc
      qmk
      postgresql
      libusb1 # for Xbox controller
      libtool # for Emacs vterm

      # Screenshot and recording tools
      flameshot
      simplescreenrecorder

      # Text and terminal utilities
      emote # Emoji picker
      feh # Manage wallpapers
      screenkey
      tree
      unixtools.ifconfig
      unixtools.netstat
      xclip # For the org-download package in Emacs
      xwininfo # Provides a cursor to click and learn about windows; moved to top-level
      xrandr

      # File and system utilities
      inotify-tools # inotifywait, inotifywatch - For file system events
      i3lock-fancy-rapid
      libnotify
      ledger-live-desktop
      playerctl # Control media players from command line
      pcmanfm # Our file browser
      sqlite
      xdg-utils

      # Other utilities
      yad # I use yad-calendar with polybar
      xdotool
      #google-chrome

      # PDF viewer
      zathura

      # Music and entertainment
      spotify

      # VR
      #immersed
    ]);
    ######## STUPID PACKAGES BULLSHIT ABOVE THIS LINE
  };
}
