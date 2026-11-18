# The Go builder for mcpx's own derivations (package.nix, docs.nix).
#
# go.mod's `go` directive follows the newest Go patch release, because
# dependencies raise it (tailscale.com needs 1.27.4). The pinned nixpkgs
# can lag a patch release behind, and a sandboxed build cannot download a
# newer toolchain, so the build would refuse to start. When the pinned
# go_1_27 is older than `wanted`, this builds that patch release from
# Go's own source with the same nixpkgs recipe; once nixpkgs catches up
# the override drops away by itself and the stock buildGo127Module is
# used. Keep `wanted` equal to go.mod's `go` line.
#
# The source hash is the one nixpkgs carries for the same release
# (pkgs/development/compilers/go/1.27.nix on the staging branch).
{
  lib,
  fetchurl,
  go_1_27,
  buildGo127Module,
}:
let
  wanted = "1.27.4";
  behind = lib.versionOlder go_1_27.version wanted;
  go = go_1_27.overrideAttrs (_: {
    version = wanted;
    src = fetchurl {
      url = "https://go.dev/dl/go${wanted}.src.tar.gz";
      hash = "sha256-c8+4VxrsDAEXdofbhxY9Rqog6XeWcncokaoXZU2aYJk=";
    };
  });
in
if behind then buildGo127Module.override { inherit go; } else buildGo127Module
