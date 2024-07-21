{
  pkgs ? import <nixpkgs> { },
}:

let
  stdenv = pkgs.stdenv;
  lib = pkgs.lib;
in

stdenv.mkDerivation rec {
  name = "tailwindcss-${version}";
  version = "3.2.7";

  src = pkgs.fetchurl {
    url = "https://github.com/tailwindlabs/tailwindcss/releases/download/v3.2.7/tailwindcss-linux-x64";
    sha256 = "35e4fa253af4ddab73490b7443b7d08f0c664a8d8b3b878eadcbb54a7e0647f8";
  };

  phases = [
    "installPhase"
    "patchPhase"
  ];

  installPhase = ''
    mkdir -p $out/bin
    install -m755 -D $src $out/bin/tailwindcss
  '';

  meta = with lib; {
    homepage = "https://github.com/tailwindlabs/tailwindcss/releases/tag/v3.2.7";
    description = "tailwindcss";
    platforms = platforms.linux;
  };
}
