# The Archer Slack collector as a flake package — issue #2137 S2.
#
# Packages the S1 collector (collector.py, beside this file) with the
# interpreter and its single dependency (websockets — requirements.txt
# pins the PoC-proven 17.3 for the pip form; here the interpreter
# carries nixpkgs' websockets build, and the collector uses only its
# stable connect / async-iterate surface), plus the service launcher
# (launcher.sh: token by name via the fleet secrets route, then exec).
#
# The systemd unit lives in nixos/services/archer-slack, which
# callPackages this file; lib/default.nix's make-archer-slack exposes
# the same package as `nix build .#archer-slack-collector`, the
# package-level check (the #8171 app + derivation + service surface).
{
  runCommand,
  python3,
}:
let
  python = python3.withPackages (ps: [ ps.websockets ]);
in
runCommand "archer-slack-collector-0.1.0"
  {
    meta = {
      description = "Archer Slack Socket Mode collector — event-driven listener appending watched-channel events to the puller store (#2137)";
      mainProgram = "archer-slack-collector";
    };
  }
  ''
    mkdir -p $out/bin $out/libexec/archer-slack-collector
    cp ${./collector.py} $out/libexec/archer-slack-collector/collector.py
    cp ${./launcher.sh} $out/bin/archer-slack-collector
    substituteInPlace $out/bin/archer-slack-collector \
      --replace-fail "@python@" "${python}/bin/python3" \
      --replace-fail "@collectorPy@" "$out/libexec/archer-slack-collector/collector.py"
    chmod +x $out/bin/archer-slack-collector
  ''
