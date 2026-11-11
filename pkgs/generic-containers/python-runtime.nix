# Generic Python runtime image for ID and fleet services (#6076).
#
# Derived from the Lab Ops Hub crawler image
# (pkgs/lab-ops-hub/container-image.nix), which established the shape
# fleet Python services need: python3 with the service's library set,
# bash + coreutils, /bin/sh linked, an explicit PATH, and the CA
# bundle wired via SSL_CERT_FILE (Nix python ships no CA config, so
# outbound HTTPS fails without it). Unlike the lab-ops image this one
# is a flake package — it uses this repo's pinned nixpkgs instead of
# that file's standalone fetchTarball pin — and it carries no service
# code: the app is layered on or mounted at run time. The library set
# defaults to the one fleet Python service that exists today (the
# crawler's idna); services needing more pass `extraPythonPackages`.
#
# Build: nix build .#generic-python-runtime
{
  dockerTools,
  python3,
  bash,
  coreutils,
  cacert,
  tzdata,
  extraPythonPackages ? (ps: [ ps.idna ]),
}:
let
  python = python3.withPackages extraPythonPackages;
in
dockerTools.buildLayeredImage {
  name = "generic-python-runtime";
  tag = "v1";

  contents = [
    python
    bash
    coreutils
    cacert
    tzdata
  ];

  extraCommands = ''
    mkdir -p work tmp
    chmod 1777 work tmp
    ln -sf ${bash}/bin/bash bin/sh
  '';

  config = {
    Cmd = [ "${python}/bin/python3" ];
    User = "1000:1000";
    WorkingDir = "/work";
    Env = [
      "HOME=/tmp"
      "PATH=${python}/bin:${bash}/bin:${coreutils}/bin:/bin:/usr/bin"
      "SSL_CERT_FILE=${cacert}/etc/ssl/certs/ca-bundle.crt"
      "TZDIR=${tzdata}/share/zoneinfo"
    ];
  };
}
