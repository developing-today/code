# Shared Nix configuration for shell.nix and flake.nix
#
# This file ensures both environments have identical packages, environment
# variables, and shell hooks. The flake.lock provides exact version pinning.
#
# Usage in flake.nix:
#   nixCommon = import ./nix-common.nix { inherit pkgs; };
#
# Usage in shell.nix:
#   nixCommon = import ./nix-common.nix { inherit pkgs; };
#
# Package management: add dev dependencies here only (never directly to
# shell.nix or flake.nix). Both shells and the system profile mirror this list.

{ pkgs }:

let
  # Rust toolchain, pinned by pkgs/id/rust-toolchain.toml and resolved through
  # rust-overlay. This is the modern way to get a complete toolchain: the
  # file's `components` list is what places clippy, rust-analyzer, rustfmt and
  # llvm-tools in the toolchain, and `channel` (currently 1.99.0) is the single
  # place the version lives. Nothing else needs to be added for those tools.
  rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./pkgs/id/rust-toolchain.toml;

  # mcpx (MCP gateway / Code Mode runner) is built from this repository. It is
  # callPackaged here so `nix develop` and `nix-shell` both get the same build,
  # the same way nixos/environment/default.nix pulls it in system-wide.
  mcpx = pkgs.callPackage ./pkgs/mcpx/package.nix { };

  # Android SDK for building Android projects (notably the OpenChamber mobile
  # app) from either shell. Mirrors nixos/environment/default.nix exactly, so
  # the dev shells and the system profile expose the same SDK. Emulator,
  # sources and system images are deliberately excluded (large, and the build
  # host does not need them). Requires allowUnfree + android_sdk.accept_license
  # on the pkgs passed in -- the flake devShell and shell.nix both set them.
  androidSdk = pkgs.androidenv.composeAndroidPackages {
    cmdLineToolsVersion = "latest";
    buildToolsVersions = [ "35.0.0" ];
    platformVersions = [ "35" ];
    includeEmulator = false;
    includeSources = false;
    includeSystemImages = false;
  };

  # Formatter binaries (keep in sync with treefmt.toml).
  # Used by the formatter wrapper (nix fmt), nix flake checks, and devShell PATH.
  fmtBins = [
    # Rust toolchain (includes rustfmt and clippy)
    rustToolchain
  ]
  ++ (with pkgs; [
    # Formatter orchestrator
    treefmt
    # Formatters and linters (keep in sync with treefmt.toml)
    nixfmt
    statix
    biome
    prettier # nodepackages remoed 2026-04-03
    shfmt
    shellcheck
    ruff
    rufo
    elmPackages.elm-format
    go_1_27 # highest released Go; also provides gofmt, the treefmt Go formatter
    haskellPackages.ormolu
    taplo
    # Utilities needed by formatter wrapper
    file
    just
    gnused
    findutils
    bash
  ]);

  # Everything else the repository's workflows reference. Grouped so it is
  # obvious why each package is here.
  devTools = with pkgs; [
    # --- NixOS management (just switch -> lib/rebuild-*.sh, lib/auth.sh) ---
    nix # nix / nix flakes / flake locking
    nix-output-monitor # the `nom` used to filter nixos-rebuild --json output
    home-manager
    git
    jq # `jq` in rebuild-simple.sh and the justfile
    sops
    ssh-to-age
    gnupg
    age

    # --- Rust (this repo's crates + the id sub-project) ---
    # The toolchain itself (rustc, cargo, rustfmt, clippy, rust-analyzer,
    # llvm-tools) is `rustToolchain` above, added via fmtBins.
    pkg-config
    openssl
    cmake # id: libsql-ffi builds its bundled SQLite through cmake
    zig # id: builds the roc-world wasm host (build.zig.zon wants 0.16)
    cargo-watch
    cargo-nextest
    cargo-llvm-cov
    cargo-audit
    cargo-outdated
    cargo-machete
    cargo-edit

    # --- Go (mcpx) ---
    # The go_1_27 toolchain is in fmtBins (it also provides gofmt); mcpx is the
    # repository's Go program, built with the same toolchain (buildGo127Module).
    # golangci-lint and actionlint are the mcpx CI lint leg's tools (#1996):
    # `nix develop` is where their versions are pinned, so the workflow passes
    # no per-action version input of its own.
    mcpx
    golangci-lint
    actionlint

    # --- Git hooks (lefthook.yml: treefmt pre-commit, conform commit-msg) ---
    lefthook
    conform

    # --- Web (id web UI) ---
    # Playwright browsers deliberately stay in pkgs/id's own shell, which wires
    # PLAYWRIGHT_BROWSERS_PATH; installing them here would be dead weight.
    bun # JavaScript bundler and runtime (required for id web builds)
    nodejs # npm, npx
    typescript # TypeScript for type checking

    # --- Assorted workflow tools referenced by justfile/scripts ---
    ripgrep
    fd
    tokei
    hyperfine
    uv
    deadnix # manual nix linter (kept out of treefmt on purpose)
  ];

  nativeBuildInputs = fmtBins ++ devTools;
in
{
  inherit rustToolchain fmtBins nativeBuildInputs;

  NIX_CONFIG = "extra-experimental-features = nix-command flakes ca-derivations";
  # Anchor bare treefmt to current directory (for ad-hoc devshell use)
  # (just recipes and nix fmt use explicit --config-file/--tree-root instead)
  TREEFMT_TREE_ROOT_CMD = "pwd";

  # Build inputs (libraries for Rust compilation)
  buildInputs = with pkgs; [ openssl ];

  # OpenSSL environment variables
  opensslEnv = {
    OPENSSL_DIR = "${pkgs.openssl.dev}";
    OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
    OPENSSL_INCLUDE_DIR = "${pkgs.openssl.dev}/include";
    PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
  };

  # Packages for shell.nix / nix develop (nativeBuildInputs + extra runtime deps)
  packages = nativeBuildInputs ++ [
    (pkgs.python3.withPackages (
      python-pkgs: with python-pkgs; [
        pydbus
        dbus-python
        pygobject3
        dbus-python
      ]
    ))
    pkgs.gobject-introspection
    pkgs.glib

    # Android: the composed SDK (cmdline-tools, platform-tools, build-tools
    # 35.0.0, platforms;android-35) and JDK 21 for Gradle 8.x. android-tools adds
    # adb/fastboot directly to PATH; ANDROID_HOME/ANDROID_SDK_ROOT/JAVA_HOME are
    # exported in shellHook below. Keep in sync with nixos/environment/default.nix.
    pkgs.jdk21
    pkgs.android-tools
    androidSdk.androidsdk
  ];

  # Shared shell hook for both nix-shell and nix develop
  shellHook = ''
    # nix develop reconstructs PATH, losing HM session paths.
    # Restore them: devshell → HM → system
    # Guard: only do PATH reordering once (multiple use flake in .envrc)
    if [ -z "$__DEVSHELL_HM_PATH_DONE" ]; then
      export __DEVSHELL_HM_PATH_DONE=1

      # Source HM session vars and extract any PATH additions
      _pre_profile_path="$PATH"
      unset __HM_SESS_VARS_SOURCED
      . "$HOME/.profile" 2>/dev/null || true
      _hm_paths="''${PATH%:$_pre_profile_path}"
      [ "$_hm_paths" = "$PATH" ] && _hm_paths=""

      # Separate devshell from system using NIX_PROFILES (NixOS env var)
      # Non-NixOS: NIX_PROFILES is unset, everything stays in devshell, PATH unchanged
      _devshell_only=""
      _system_only=""
      _remaining="$_pre_profile_path"
      while [ -n "$_remaining" ]; do
        _e="''${_remaining%%:*}"
        [ "$_e" = "$_remaining" ] && _remaining="" || _remaining="''${_remaining#*:}"
        _is_sys=0
        for _p in $NIX_PROFILES; do
          case "$_e" in "$_p"/*) _is_sys=1; break ;; esac
        done
        if [ "$_is_sys" = 1 ]; then
          _system_only="$_system_only''${_system_only:+:}$_e"
        else
          _devshell_only="$_devshell_only''${_devshell_only:+:}$_e"
        fi
      done

      # Reconstruct: devshell → HM → system
      export PATH="$_devshell_only''${_hm_paths:+:$_hm_paths}''${_system_only:+:$_system_only}"
      unset _pre_profile_path _hm_paths _devshell_only _system_only _remaining _e _is_sys _p
    fi
    # Android SDK + JDK 21. Exported here (not as a mkShell attr) so both
    # shell.nix and the flake devShell pick them up through the shared hook.
    # The OpenChamber mobile scripts read ANDROID_HOME and fall back to a
    # Homebrew JAVA_HOME when it is unset, so JAVA_HOME must be exported.
    export ANDROID_HOME="${androidSdk.androidsdk}/libexec/android-sdk"
    export ANDROID_SDK_ROOT="$ANDROID_HOME"
    export JAVA_HOME="${pkgs.jdk21.home}"
    export PATH="$JAVA_HOME/bin:$ANDROID_HOME/platform-tools:$PATH"

    # mcpx daemon autostart
    if command -v mcpx &>/dev/null; then
      mcpx daemon start 2>/dev/null || true
    fi

    echo ""
    just --list
    echo ""
    echo "Welcome to the development shell!"
    echo ""
  '';
}
