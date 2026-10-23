{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  makeWrapper,
  unzip,
  python3,
  zlib,
  ncurses,
  libxml2,
  libffi,
  libbsd,
}:

# Mojo, Modular's AI/systems programming language.
#
# Packaging notes:
#   * Upstream ships prebuilt manylinux wheels; there is no practical source
#     build. The compiler (/KGEN in modular/modular) was open sourced under
#     Apache-2.0-with-LLVM-exceptions, but it builds via a Bazel + LLVM monorepo
#     and upstream does not accept compiler contributions, so building it from
#     source is not currently viable.
#   * We track the *stable* PyPI release rather than the nightly Artifact
#     Registry feed, because nightly wheels are garbage collected and their
#     pinned hashes rot.
#   * Derived from the (unreviewed, since-abandoned) nixpkgs PR #523845, updated
#     from the 1.0.0b2 nightly to stable 1.0.0 and extended to aarch64.
let
  version = "1.0.0";
  mblackVersion = "26.5.0";

  pythonEnv = python3.withPackages (
    ps: with ps; [
      click
      mypy-extensions
      pathspec
      platformdirs
    ]
  );

  pythonPath = lib.makeSearchPath python3.sitePackages [ pythonEnv ];

  # Upstream links against a non-unicode, termlib-split ncurses 6 ABI.
  ncursesCompat = ncurses.override {
    abiVersion = "6";
    unicodeSupport = false;
    withTermlib = true;
  };

  # Platform-specific wheels. Keys are Nix system strings.
  wheelsFor = {
    "x86_64-linux" = {
      tag = "manylinux_2_34_x86_64";
      mojo = "sha256-cl1rfymlozNOPCJfNfK3Cf1GFRoKMaLCJzixk338g9k=";
      mojoCompiler = "sha256-6eYPljjmnKD0vnKSRoUj/JihQ/WNv5Ak9g7Wi4dKhn4=";
      mojoLldbLibs = "sha256-3hUch/P7TRhKhec16uBb9D+anetMuPKwfPa1nLdp2T8=";
      mojoUrl = "https://files.pythonhosted.org/packages/c4/89/326637e71282288e7d3a8ef2990dbf9fbfb07c1b3d21b836ffe2ee3111a9/mojo-1.0.0-py3-none-manylinux_2_34_x86_64.whl";
      mojoCompilerUrl = "https://files.pythonhosted.org/packages/f0/5f/f38fefe327d1c81e28def69c4a52ae4f75e389cb6e613a2c04ca8d68d582/mojo_compiler-1.0.0-py3-none-manylinux_2_34_x86_64.whl";
      mojoLldbLibsUrl = "https://files.pythonhosted.org/packages/12/a0/8280a1869d017102d50e0dcf1962e0bc59834f8fe07b435e1a575bb5c923/mojo_lldb_libs-1.0.0-py3-none-manylinux_2_34_x86_64.whl";
    };
    "aarch64-linux" = {
      tag = "manylinux_2_34_aarch64";
      mojo = "sha256-ptsYsoRuxId8XNQ6Lc4VVkAFINwNRFDci0cD2rVp6aI=";
      mojoCompiler = "sha256-OuPrDFiolW9ULjJPc2hm3wri+fh16KJ5y+6pYexOnug=";
      mojoLldbLibs = "sha256-frHyBIKfAx/ajMnuTn9GHxyj3fHI+1Cavy628Cyd5PU=";
      mojoUrl = "https://files.pythonhosted.org/packages/31/e3/d429fb53aa018e592dc378def8a92c67d0978078c8c30d089019195a285f/mojo-1.0.0-py3-none-manylinux_2_34_aarch64.whl";
      mojoCompilerUrl = "https://files.pythonhosted.org/packages/a5/f9/cbfe2bf947d0926ad57599513d898f1e02ee0c60af1253b779ecaa810235/mojo_compiler-1.0.0-py3-none-manylinux_2_34_aarch64.whl";
      mojoLldbLibsUrl = "https://files.pythonhosted.org/packages/38/e1/a7c6c73c875f5692587beda9ca68773bae069d60b1c79d9fffd8a2840f4d/mojo_lldb_libs-1.0.0-py3-none-manylinux_2_34_aarch64.whl";
    };
  };

  w =
    wheelsFor.${stdenv.hostPlatform.system}
      or (throw "mojo: unsupported platform ${stdenv.hostPlatform.system}");
in
stdenv.mkDerivation {
  pname = "mojo";
  inherit version;

  __structuredAttrs = true;

  srcs = [
    (fetchurl {
      name = "mojo-${version}-py3-none-${w.tag}.whl";
      url = w.mojoUrl;
      hash = w.mojo;
    })
    (fetchurl {
      name = "mojo_compiler-${version}-py3-none-${w.tag}.whl";
      url = w.mojoCompilerUrl;
      hash = w.mojoCompiler;
    })
    (fetchurl {
      name = "mojo_compiler_mojo_libs-${version}-py3-none-any.whl";
      url = "https://files.pythonhosted.org/packages/54/99/ea401ff1db56a4af8607283b95627e01b986fc67f510715b07f100118105/mojo_compiler_mojo_libs-1.0.0-py3-none-any.whl";
      hash = "sha256-IKkuN+y9GeLbsaUlYSqN5MnyZvVTbe9VxeKweGMg6LM=";
    })
    (fetchurl {
      name = "mblack-${mblackVersion}-py3-none-any.whl";
      url = "https://files.pythonhosted.org/packages/f0/36/8147d0627cde9043557f7906eabe64ff6295ec910dade5251e5703f0f6d9/mblack-26.5.0-py3-none-any.whl";
      hash = "sha256-ByswRkbCd5eeah0ZsIbvImu3OBgQkC6XXPHDiG0deIM=";
    })
    (fetchurl {
      name = "mojo_lldb_libs-${version}-py3-none-${w.tag}.whl";
      url = w.mojoLldbLibsUrl;
      hash = w.mojoLldbLibs;
    })
  ];

  dontUnpack = true;
  strictDeps = true;

  nativeBuildInputs = [
    autoPatchelfHook
    makeWrapper
    unzip
  ];

  buildInputs = [
    stdenv.cc.cc.lib
    zlib
    ncursesCompat
    libxml2
    libffi
    libbsd
  ];

  appendRunpaths = [ "${placeholder "out"}/${python3.sitePackages}/modular/lib" ];

  installPhase = ''
    runHook preInstall

    sitePackages="$out/${python3.sitePackages}"
    mkdir -p "$sitePackages" "$out/bin"

    for wheel in "''${srcs[@]}"; do
      unzip -q "$wheel" -d "$sitePackages"
    done

    for dataDir in "$sitePackages"/*.data/platlib; do
      if [ -d "$dataDir" ]; then
        cp -R "$dataDir"/* "$sitePackages/"
      fi
    done

    rm -rf "$sitePackages"/*.data
    chmod -R u+w "$sitePackages"

    modularRoot="$sitePackages/modular"
    wrapperArgs=(
      --prefix PYTHONPATH : "$sitePackages:${pythonPath}"
      --set MODULAR_MAX_PACKAGE_ROOT "$modularRoot"
      --set MODULAR_MOJO_MAX_PACKAGE_ROOT "$modularRoot"
      --set MODULAR_MOJO_MAX_DRIVER_PATH "$modularRoot/bin/mojo"
      --set MODULAR_MOJO_MAX_IMPORT_PATH "$modularRoot/lib/mojo"
      --set MODULAR_MOJO_MAX_MBLACK_PATH "$out/bin/mblack"
      # Upstream PR never set this, leaving mojo to hunt for its linker on PATH.
      --set MODULAR_MOJO_MAX_LLD_PATH "$modularRoot/bin/lld"
      # Opt out of telemetry/crash reporting by default (also silences the
      # "Failed to initialize Crashpad" warning). Override by exporting
      # MODULAR_TELEMETRY_ENABLED=true.
      --set-default MODULAR_TELEMETRY_ENABLED "false"
      --set MODULAR_MOJO_MAX_LLDB_PLUGIN_PATH "$modularRoot/lib/libMojoLLDB.so"
      --set MODULAR_MOJO_MAX_LLDB_VISUALIZERS_PATH "$modularRoot/lib/lldb-visualizers"
    )

    find "$modularRoot/bin" -type f -exec chmod +x {} +
    find "$modularRoot/lib" -type f -name '*.so*' -exec chmod +x {} +

    makeBinaryEntrypoint() {
      makeWrapper "$2" "$out/bin/$1" "''${wrapperArgs[@]}"
    }

    makePythonEntrypoint() {
      local name="$1" module="$2" function="$3"
      printf '%s\n' \
        "#!${pythonEnv}/bin/python" \
        "import sys" \
        "from $module import $function" \
        "sys.exit($function())" \
        > "$out/bin/$name"
      chmod +x "$out/bin/$name"
      wrapProgram "$out/bin/$name" "''${wrapperArgs[@]}"
    }

    makeBinaryEntrypoint mojo "$modularRoot/bin/mojo"
    makeBinaryEntrypoint lld "$modularRoot/bin/lld"
    makeBinaryEntrypoint modular-crashpad-handler "$modularRoot/bin/modular-crashpad-handler"
    makePythonEntrypoint mblack mblack patched_main
    makePythonEntrypoint gpu-query _mojo._entrypoints exec_gpu_query
    makePythonEntrypoint lldb-argdumper _mojo._entrypoints exec_lldb_argdumper
    makePythonEntrypoint lldb-dap _mojo._entrypoints exec_lldb_dap
    makePythonEntrypoint lldb-server _mojo._entrypoints exec_lldb_server
    makePythonEntrypoint llvm-symbolizer _mojo._entrypoints exec_llvm_symbolizer
    makePythonEntrypoint mojo-lldb _mojo._entrypoints exec_mojo_lldb
    makePythonEntrypoint mojo-lsp-server _mojo._entrypoints exec_mojo_lsp_server

    runHook postInstall
  '';

  doInstallCheck = true;

  installCheckPhase = ''
        runHook preInstallCheck

        export HOME="$PWD/home"
        export XDG_CACHE_HOME="$HOME/.cache"
        export XDG_DATA_HOME="$HOME/.local/share"
        mkdir -p "$XDG_CACHE_HOME" "$XDG_DATA_HOME"

        "$out/bin/mojo" --version
        cat > hello.mojo <<'EOF'
    def main():
        print("Hello, Mojo")
    EOF
        "$out/bin/mojo" hello.mojo
        "$out/bin/mojo-lldb" --version

        runHook postInstallCheck
  '';

  meta = {
    description = "Programming language for AI developers";
    homepage = "https://mojolang.org";
    changelog = "https://docs.modular.com/mojo/changelog/";
    sourceProvenance = with lib.sourceTypes; [ binaryNativeCode ];
    # Wheels ship under LicenseRef-MAX-Platform-Software-License (Modular
    # Community License), even though the compiler sources are Apache-2.0.
    license = lib.licenses.unfree;
    mainProgram = "mojo";
    platforms = [
      "x86_64-linux"
      "aarch64-linux"
    ];
  };
}
