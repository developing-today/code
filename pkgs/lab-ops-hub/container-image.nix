# Daemon-less OCI image for the Lab Ops Hub crawler container.
# nixpkgs is pinned to the same rev as dezren39/nix's flake.lock.
# Build:  nix build -f container-image.nix  -> store path is a docker-archive tar
# Push:   crane push <tar> registry.cloudflare.com/<account>/lab-ops-hub-crawlercontainer:latest
# (or load it anywhere with docker/podman and `wrangler deploy` from there)
let
  nixpkgs = fetchTarball {
    url = "https://github.com/nixos/nixpkgs/archive/f20cd90e28fe0403873f473b29e42662cd86e8c4.tar.gz";
    sha256 = "sha256-EjZqHKObBfIOeHZnMRVV87HB3pPq/WBkmRJWyFxmZu8=";
  };
  pkgs = import nixpkgs { };
in
pkgs.dockerTools.buildLayeredImage {
  name = "lab-ops-hub-crawlercontainer";
  tag = "latest";
  contents = [ pkgs.python3 pkgs.cacert ];
  extraCommands = ''
    mkdir -p app
    cp ${./container/app.py} app/app.py
  '';
  config = {
    WorkingDir = "/app";
    Cmd = [ "python3" "app.py" ];
    ExposedPorts = { "8080/tcp" = { }; };
    # Nix python ships no CA bundle config; without this, outbound HTTPS
    # fails CERTIFICATE_VERIFY_FAILED inside the container.
    Env = [ "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt" ];
  };
}
