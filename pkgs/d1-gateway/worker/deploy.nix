{
  writeShellApplication,
  curl,
  git,
  jq,
  nodejs,
  wrangler,
  d1Gateway,
}:
# Deploys d1-gateway through the Cloudflare versions flow: wrangler
# `versions upload` (tagged with the commit), then `versions deploy`
# of that version at 100%, then waits for /health to report it.
writeShellApplication {
  name = "d1-gateway-deploy";
  runtimeInputs = [
    curl
    git
    jq
    nodejs
    wrangler
  ];
  text = ''
    export D1_GATEWAY_PKG="${d1Gateway}"
    exec bash ${./deploy.sh} "$@"
  '';
}
