# Generic runtime base image for ID and fleet services (#6076).
#
# The common denominator of the service-specific images in this repo
# (pkgs/mcpx, pkgs/identity, pkgs/lab-ops-hub): a small POSIX userspace,
# a CA bundle wired up via SSL_CERT_FILE, tzdata, and writable /tmp and
# /work directories. A service artifact (a static binary, a script, an
# app directory) is layered on or mounted at run time; this image
# deliberately carries no service code of its own.
#
# Build: nix build .#generic-runtime
{
  dockerTools,
  bash,
  coreutils,
  cacert,
  tzdata,
}:
dockerTools.buildLayeredImage {
  name = "generic-runtime";
  tag = "v1";

  contents = [
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
    Cmd = [ "${bash}/bin/bash" ];
    User = "1000:1000";
    WorkingDir = "/work";
    Env = [
      "HOME=/tmp"
      "PATH=${bash}/bin:${coreutils}/bin:/bin:/usr/bin"
      "SSL_CERT_FILE=${cacert}/etc/ssl/certs/ca-bundle.crt"
      "TZDIR=${tzdata}/share/zoneinfo"
    ];
  };
}
