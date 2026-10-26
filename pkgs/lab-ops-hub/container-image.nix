# Daemon-less OCI image for the Lab Ops Hub crawler container.
# nixpkgs is pinned to the same rev as dezren39/nix's flake.lock.
# Build:  nix build -f container-image.nix  -> store path is a docker-archive tar
# Push:   crane push <tar> registry.cloudflare.com/<account>/lab-ops-hub-crawlercontainer:v5
# (or load it anywhere with docker/podman and `wrangler deploy` from there)
#
# Image shape matters (learned 2026-10-25): the v3 shape — relative Cmd
# ("python3 app.py"), no PATH env, no shell — stopped starting after
# Cloudflare's 2026-10-20 platform changes, while conventionally-shaped
# images (e.g. Cloudflare's hello-world) start fine. This recipe uses the
# absolute interpreter path, sets PATH explicitly, and ships bash +
# coreutils with /bin/sh linked.
let
  nixpkgs = fetchTarball {
    url = "https://github.com/nixos/nixpkgs/archive/f20cd90e28fe0403873f473b29e42662cd86e8c4.tar.gz";
    sha256 = "sha256-EjZqHKObBfIOeHZnMRVV87HB3pPq/WBkmRJWyFxmZu8=";
  };
  pkgs = import nixpkgs { };
in
pkgs.dockerTools.buildLayeredImage {
  name = "lab-ops-hub-crawlercontainer";
  tag = "v5";
  contents = [
    pkgs.python3
    pkgs.cacert
    pkgs.bash
    pkgs.coreutils
  ];
  extraCommands = ''
    mkdir -p app
    cp ${./container/app.py} app/app.py
    ln -sf ${pkgs.bash}/bin/bash bin/sh
  '';
  config = {
    WorkingDir = "/app";
    Cmd = [
      "${pkgs.python3}/bin/python3"
      "/app/app.py"
    ];
    ExposedPorts = {
      "8080/tcp" = { };
    };
    Env = [
      "PATH=${pkgs.python3}/bin:${pkgs.coreutils}/bin:${pkgs.bash}/bin:/bin:/usr/bin"
      # Nix python ships no CA bundle config; without this, outbound HTTPS
      # fails CERTIFICATE_VERIFY_FAILED inside the container.
      "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
    ];
  };
}
