{
  dockerTools,
  mcpx,
  bun,
  cacert,
}:
dockerTools.buildLayeredImage {
  name = "mcpx";
  tag = mcpx.version;

  # The binary's runtime closure is deliberately small: mcpx plus the one
  # JavaScript runtime the image promises to support. Do not use the wrapped
  # workstation package here; that wrapper adds Deno, Node and git.
  contents = [
    mcpx
    bun
    cacert
  ];

  extraCommands = ''
    mkdir -p work tmp
    chmod 1777 work tmp
  '';

  config = {
    Entrypoint = [ "${mcpx}/bin/mcpx" ];
    Cmd = [ "--help" ];
    User = "1000:1000";
    WorkingDir = "/work";
    Env = [
      "HOME=/tmp"
      "MCPX_STATE_DIR=/tmp/mcpx-state"
      "MCPX_CACHE_DIR=/tmp/mcpx-cache"
      "MCPX_SCRIPT_RUNTIME=bun"
      "PATH=${bun}/bin:${mcpx}/bin"
      "SSL_CERT_FILE=${cacert}/etc/ssl/certs/ca-bundle.crt"
    ];
  };
}
