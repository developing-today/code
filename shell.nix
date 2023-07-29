{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell rec {

  buildInputs = with pkgs; [
    clang
    cmake
    gcc
    libiconv
    llvmPackages.bintools
    openjdk19
    openssl
    pkg-config
    pkgconfig
    postgresql
    rustup
    sqlite
    zlib
  ];

  RUSTC_VERSION = pkgs.lib.readFile ./rust-toolchain;

  LIBCLANG_PATH = pkgs.lib.makeLibraryPath [ pkgs.llvmPackages_latest.libclang.lib ];

  shellHook = ''
    openssl_lib_path=/nix/store/xal21vd4d9nfwjkcvw0fyq6ivsbxg1pz-openssl-3.0.9/lib
    export LD_LIBRARY_PATH=$LD_LIBRARY_PATH:$openssl_lib_path
    export PATH=$PATH:''${CARGO_HOME:-~/.cargo}/bin
    export PATH=$PATH:''${RUSTUP_HOME:-~/.rustup}/toolchains/$RUSTC_VERSION-x86_64-unknown-linux-gnu/bin/
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
}
