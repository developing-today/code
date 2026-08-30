{
  inputs,
  system,
  pkgs,
  lib,
  ...
}:
let
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
  inherit (inputs.opencode.packages.${system}) opencode-desktop;
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
      opencode-desktop
    ]
    ++ (with inputs; [
      #rose-pine-hyprcursor.packages.${pkgs.system}.default
      nix-output-monitor.packages.${system}.default
      ssh-to-age.packages.${system}.default
      nix-search.packages.${system}.default
      zen-browser.packages.${system}.default
      #hyprland-qtutils.packages.${system}.hyprland-qtutils
      clan-core.packages.${system}.clan-cli
      opencode.packages.${system}.opencode
      # OMP ("Oh My Pi") harness, from https://omp.sh -> github:can1357/oh-my-pi
      oh-my-pi.packages.${system}.omp
    ])
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
    ])
    ++ (with pkgs; [
      # AI coding agents / LLM CLIs.
      # NOTE: opencode is installed above from its own flake input, and
      # oh-my-posh comes from programs.oh-my-posh in home/common/default.nix.
      claude-code # Anthropic Claude Code
      codex # OpenAI Codex CLI
      openclaw # OpenClaw (openclaw.ai) assistant
      gemini-cli # Google Gemini
      qwen-code # Alibaba Qwen Code
      crush # Charm agentic coder
      amp-cli # Sourcegraph Amp
      goose-cli # Block Goose
      aider-chat # Aider pair programmer
      cursor-cli # Cursor agent
      aichat # multi-provider LLM CLI
      mods # Charm LLM pipe tool
      tgpt # no-auth LLM CLI
    ])
    ++ [
      # Mojo (Modular). Prebuilt upstream wheels, unfree license.
      # See pkgs/mojo/default.nix for why we track stable PyPI over nightly.
      (pkgs.callPackage ../../pkgs/mojo { })
      # Agent harnesses not packaged in nixpkgs. OMP comes from its own
      # upstream flake input above; these two we package ourselves.
      (pkgs.callPackage ../../pkgs/hermes-agent { }) # https://hermes-agent.nousresearch.com
      (pkgs.callPackage ../../pkgs/pi-coding-agent { }) # https://pi.dev
    ]
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
    ++ (with inputs.nixpkgs-unstable.legacyPackages.${system}; [ ])
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
      grimblast
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
      hyprshade
      hyprshot
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
