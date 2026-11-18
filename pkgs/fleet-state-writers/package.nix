# Preserve the relative helper imports when deploying either operator writer.
{ stdenvNoCC, python3, makeWrapper }:
stdenvNoCC.mkDerivation {
  pname = "fleet-state-writers";
  version = "2026-11-18";
  dontUnpack = true;
  nativeBuildInputs = [ makeWrapper ];
  installPhase = ''
    root=$out/libexec/fleet-state-writers
    mkdir -p $root/scripts $root/pm-stack/shared-schema $out/bin
    cp ${../../scripts/fleet_turso.py} $root/scripts/fleet_turso.py
    cp ${../../scripts/ainventory-interpretation.py} $root/scripts/ainventory-interpretation.py
    cp ${../../pm-stack/shared-schema/orchestration-writer.py} $root/pm-stack/shared-schema/orchestration-writer.py
    makeWrapper ${python3}/bin/python3 $out/bin/orchestration-writer \
      --add-flags "$root/pm-stack/shared-schema/orchestration-writer.py"
    makeWrapper ${python3}/bin/python3 $out/bin/ainventory-interpretation \
      --add-flags "$root/scripts/ainventory-interpretation.py"
  '';
}
