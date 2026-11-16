# sigterm-witness -- signal-attribution wrapper for the PM services
# (code#8777). A tiny C program: install an SA_SIGINFO handler, fork
# the real command, and for every signal delivered to the wrapper,
# append a receipt naming the sender (pid, uid, comm, parent pid,
# cmdline where capturable) to the --log file, forward the signal to
# the child, and finally die the child's death so supervisors see
# identical exit semantics. It exists because the laptop has no
# auditd: when an unidentified actor SIGTERMed pm-aggregator and
# pm-dispatch at 2026-11-11T22:01:09Z, nothing anywhere recorded who
# sent it. Drew's rule (2026-11-11): these services are reset by
# port-handover/takeover or an explicit named systemctl action --
# never a pattern kill; the witness makes any violation a matter of
# record.
{
  lib,
  stdenv,
}:
stdenv.mkDerivation {
  pname = "sigterm-witness";
  version = "0.1.0";

  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [ ./sigterm-witness.c ];
  };

  buildPhase = ''
    runHook preBuild
    $CC -O2 -Wall -o sigterm-witness sigterm-witness.c
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    install -Dm755 sigterm-witness $out/bin/sigterm-witness
    runHook postInstall
  '';

  meta = {
    description = "Signal-attribution wrapper: logs who signals the wrapped process, then forwards transparently";
    mainProgram = "sigterm-witness";
    platforms = lib.platforms.linux;
  };
}
