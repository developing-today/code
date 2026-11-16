{
  dockerTools,
  identity,
  cacert,
}:
dockerTools.buildLayeredImage {
  name = "identity";
  tag = identity.version;
  contents = [
    identity
    cacert
  ];

  extraCommands = ''
    mkdir -p data
  '';

  config = {
    Entrypoint = [ "${identity}/bin/charm" ];
    Cmd = [ "serve" ];
    WorkingDir = "/data";
    Volumes = {
      "/data" = { };
    };
    Env = [
      "CHARM_SERVER_DATA_DIR=/data"
      "SSL_CERT_FILE=${cacert}/etc/ssl/certs/ca-bundle.crt"
    ];
    ExposedPorts = {
      "35353/tcp" = { };
      "35354/tcp" = { };
      "35355/tcp" = { };
      "35356/tcp" = { };
    };
  };
}
