{ pkgs ? import <nixpkgs> {} }:

let
  tailwindcss = import ./tailwindcss.nix { pkgs = pkgs; };
in

pkgs.mkShell rec {
  buildInputs = with pkgs; [
    clang
    llvmPackages.bintools
    rustup
    postgresql
    libiconv
    openssl
    pkgconfig
    sqlite
    cmake
    openjdk19
    nodejs
    tailwindcss
    shellcheck
    python311
    python311Packages.pip
    python311Packages.virtualenv
    docker
    jq
    yq
  ];

  RUSTC_VERSION = pkgs.lib.readFile ./rust-toolchain;

  LIBCLANG_PATH = pkgs.lib.makeLibraryPath [ pkgs.llvmPackages_latest.libclang.lib ];

  shellHook = ''
      export PATH=$PATH:''${CARGO_HOME:-~/.cargo}/bin
      export PATH=$PATH:''${RUSTUP_HOME:-~/.rustup}/toolchains/$RUSTC_VERSION-x86_64-unknown-linux-gnu/bin/
      source ./export-lib.sh > /dev/null
      printf "G'day, $(whoami)!\\n"
    '';

  RUSTFLAGS = (builtins.map (a: ''-L ${a}/lib'') []);

  BINDGEN_EXTRA_CLANG_ARGS =
    (builtins.map (a: ''-I"${a}/include"'') [
      pkgs.glibc.dev
    ])
    ++ [
      ''-I"${pkgs.llvmPackages_latest.libclang.lib}/lib/clang/${pkgs.llvmPackages_latest.libclang.version}/include"''
      ''-I"${pkgs.glib.dev}/include/glib-2.0"''
      ''-I${pkgs.glib.out}/lib/glib-2.0/include/''
    ];

  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [ pkgs.openssl ];
}