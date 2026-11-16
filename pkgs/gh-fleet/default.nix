# gh-fleet — `gh` that routes through `fleet gh` by default
# (Drew, 2026-11-14; the laptop/Nix counterpart of the sandbox's
# ~/tools/gh-shim.sh).
#
# The profile's `gh` becomes a thin shim: every call execs
# `fleet gh "$@"`, and fleet resolves the real gh through
# FLEET_GH_BIN, pinned here to the pristine package's store path —
# so fleet's own gh calls can never PATH-resolve back into this
# shim (no recursion). FLEET_GH_SHIM_ACTIVE is a second guard:
# anything nested under the shim gets the pristine binary.
#
# Escape hatches, by design:
#   * `gh-orig`            the pristine gh, one name away;
#   * `fleet gh --raw …` / FLEET_GH_RAW=1
#                          fleet's own no-rotation bypass, which
#                          passes straight through this shim;
#   * local-only verbs (--version, --help, -h, help, bare `gh`)
#     exec the pristine gh directly — they need no token, and
#     `gh --help` must print gh's own help, not fleet's usage.
#
# Everything else the pristine package ships (man pages,
# completions) is linked through untouched.
{
  lib,
  runCommand,
  writeShellScript,
  gh,
  fleet,
}:

let
  shim = writeShellScript "gh" ''
    if [ "''${FLEET_GH_SHIM_ACTIVE:-}" = "1" ]; then
      exec ${gh}/bin/gh "$@"
    fi
    case "''${1:-}" in
      --version|--help|-h|help|"")
        exec ${gh}/bin/gh "$@"
        ;;
    esac
    export FLEET_GH_BIN=${gh}/bin/gh
    export FLEET_GH_SHIM_ACTIVE=1
    exec ${fleet}/bin/fleet gh "$@"
  '';

  orig = writeShellScript "gh-orig" ''
    exec ${gh}/bin/gh "$@"
  '';
in
runCommand "gh-fleet-${gh.version}"
  {
    meta = {
      description = "gh routed through fleet gh token rotation, with gh-orig as the pristine fallback";
      homepage = "https://github.com/cli/cli";
      license = lib.licenses.mit;
      mainProgram = "gh";
    };
    passthru = {
      unwrapped = gh;
    };
  }
  ''
    mkdir -p $out/bin
    cp ${shim} $out/bin/gh
    cp ${orig} $out/bin/gh-orig
    chmod +x $out/bin/gh $out/bin/gh-orig
    for f in ${gh}/bin/*; do
      if [ "$f" != "${gh}/bin/gh" ]; then
        ln -s "$f" $out/bin/
      fi
    done
    for d in ${gh}/*; do
      if [ "$(basename "$d")" != "bin" ]; then
        ln -s "$d" $out/
      fi
    done
  ''
