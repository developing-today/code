# Generic JavaScript runtime image for ID and fleet services (#6076).
#
# Derived from the mcpx image (pkgs/mcpx/container-image.nix), which
# established that fleet JS services run on bun with a deliberately
# small closure: the runtime, a POSIX shell for operational tooling,
# cacert, and nothing else. The service's own bundle is layered on or
# mounted at run time; unlike the mcpx image this one carries no
# service binary, so any bun-based ID or fleet service can use it.
#
# Build: nix build .#generic-js-runtime
{
  dockerTools,
  bun,
  bash,
  coreutils,
  cacert,
  tzdata,
}:
dockerTools.buildLayeredImage {
  name = "generic-js-runtime";
  tag = "v1";

  contents = [
    bun
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
    Cmd = [ "${bun}/bin/bun" ];
    User = "1000:1000";
    WorkingDir = "/work";
    Env = [
      "HOME=/tmp"
      "PATH=${bun}/bin:${bash}/bin:${coreutils}/bin:/bin:/usr/bin"
      "SSL_CERT_FILE=${cacert}/etc/ssl/certs/ca-bundle.crt"
      "TZDIR=${tzdata}/share/zoneinfo"
    ];
  };
}
