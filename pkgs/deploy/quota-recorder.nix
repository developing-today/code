# The sandbox quota recorder as a flake package - issue #5770 (M3).
#
# The recorder script lived loose in the sandbox's ~/tools and ran
# from hand-copied systemd units with no installer of record: every
# sandbox recycle stranded it until an agent sweep reinstalled the
# units by hand (design doc/2026-11-04_deployment-architecture-
# services-layer.md §0/§4.4). The script is carried into the repo
# byte-identical to the live ~/tools/quota-recorder.py it replaces
# (2026-11-11 revision: file-first, Turso landing per #5690) so the
# deploy registry has a real package to name and the source is
# pinned in git (cf. issue #3235's sandbox-unit source audit). The
# data path is unchanged; #5703's boundary is respected — this
# package is the deployment mechanism only.
{
  runCommand,
  makeWrapper,
  python3,
}:
runCommand "quota-recorder-0.1.0"
  {
    nativeBuildInputs = [ makeWrapper ];
    meta = {
      description = "Sandbox quota recorder (Muse subscription quota snapshots, file-first with Turso landing)";
      mainProgram = "quota-recorder";
    };
  }
  ''
    mkdir -p $out/bin $out/libexec/quota-recorder
    cp ${./quota-recorder.py} $out/libexec/quota-recorder/quota-recorder.py
    makeWrapper ${python3}/bin/python3 $out/bin/quota-recorder \
      --add-flags $out/libexec/quota-recorder/quota-recorder.py
  ''
