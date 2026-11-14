{
  lib,
  dockerTools,
  bash,
  coreutils,
  findutils,
  gnugrep,
  gnused,
  gawk,
  ucspi-tcp,
  curl,
  jq,
  cacert,
}:
let
  runtime = [
    bash
    coreutils
    findutils
    gnugrep
    gnused
    gawk
    ucspi-tcp
    curl
    jq
  ];
in
dockerTools.buildLayeredImage {
  name = "identity-provider";
  tag = "0.1.0";
  contents = runtime ++ [ cacert ];

  extraCommands = ''
    mkdir -p app
    cp -r ${./provider}/. app/
    chmod 755 app/*.sh app/pages/*.sh
    touch .env
  '';

  config = {
    Entrypoint = [ "${bash}/bin/bash" ];
    Cmd = [ "/app/start.sh" ];
    WorkingDir = "/app";
    Env = [
      "DEV=false"
      "PORT=3333"
      "PATH=${lib.makeBinPath runtime}"
      "SSL_CERT_FILE=${cacert}/etc/ssl/certs/ca-bundle.crt"
    ];
    ExposedPorts = {
      "3333/tcp" = { };
    };
  };
}
