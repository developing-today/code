# v4 of the Lab Ops Hub crawler image — conventional-shape iteration.
# Changes vs v3 (which fails to start on Cloudflare's current start path
# while the official hello-world image starts instantly):
#   * Cmd uses the ABSOLUTE interpreter path (no PATH resolution needed)
#   * PATH env is set explicitly
#   * bash + coreutils included, /bin/sh symlinked (runtime may shell out)
let
  nixpkgs = fetchTarball {
    url = "https://github.com/nixos/nixpkgs/archive/f20cd90e28fe0403873f473b29e42662cd86e8c4.tar.gz";
    sha256 = "sha256-EjZqHKObBfIOeHZnMRVV87HB3pPq/WBkmRJWyFxmZu8=";
  };
  pkgs = import nixpkgs { };
in
pkgs.dockerTools.buildLayeredImage {
  name = "lab-ops-hub-crawlercontainer";
  tag = "v4";
  contents = [ pkgs.python3 pkgs.cacert pkgs.bash pkgs.coreutils ];
  extraCommands = ''
    mkdir -p app
    cp ${./app.py} app/app.py
    ln -sf ${pkgs.bash}/bin/bash bin/sh
  '';
  config = {
    WorkingDir = "/app";
    Cmd = [ "${pkgs.python3}/bin/python3" "/app/app.py" ];
    ExposedPorts = { "8080/tcp" = { }; };
    Env = [
      "PATH=${pkgs.python3}/bin:${pkgs.coreutils}/bin:${pkgs.bash}/bin:/bin:/usr/bin"
      "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
    ];
  };
}
