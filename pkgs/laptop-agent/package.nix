# laptop-agent daemon — the reconciled single source (code#10131).
#
# Union of the live daemon lineage (timeout semantics, trip logging,
# /exec cancel, disconnect reap, /file) and the darwin-nix vendored
# lineage this package shape comes from (persisted job index,
# fail-closed startup validation, request-framing 400s, job
# summaries, spool reaping, bytes-space auth). Vendored here as the
# canonical source; the nix module that deploys the daemon consumes
# this package. See src/laptop_agent.py's header for the full
# reconciliation notes.
{
  lib,
  stdenv,
  python3,
  bash,
  makeWrapper,
}:

stdenv.mkDerivation {
  pname = "laptop-agent";
  version = "0.1.0";

  # The resident fleet execution agent (code#5343): a single
  # stdlib-only Python daemon. The sandbox session proxy routes
  # laptop-agent, secrets broker, and queue broker traffic to it and
  # its sibling brokers.
  src = ./src;

  nativeBuildInputs = [ makeWrapper ];

  dontBuild = true;

  installPhase = ''
    runHook preInstall
    install -Dm755 laptop_agent.py $out/libexec/laptop_agent.py
    makeWrapper ${python3}/bin/python3 $out/bin/laptop-agent \
      --add-flags "$out/libexec/laptop_agent.py" \
      --set LAPTOP_AGENT_BASH ${bash}/bin/bash
    runHook postInstall
  '';

  meta = {
    description = "laptop-agent — resident fleet execution agent daemon (reconciled source, code#10131)";
    mainProgram = "laptop-agent";
    platforms = lib.platforms.unix;
  };
}
