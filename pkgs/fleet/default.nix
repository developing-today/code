# fleet — the multicall fleet binary (developing-today/code#5412,
# step 1: #5429). One Rust workspace, one binary, capability
# variants selected by cargo features; the installed binary is
# always named `fleet`, and hyphenated names (fleet-serve,
# fleet-connect, repo-queue, ...) are argv0 personalities, not
# separate artifacts (design note sections 2-3, refinement A).
#
# Build through the root flake:
#
#   nix build .#fleet .#fleet-serve .#fleet-connect .#fleet-connect-static
#
# The connect variants carry the design's D7 size contract: the
# binary must stay <= 2 MiB (target <= 1.5 MiB) so it can be
# dropped onto any host. The gate is enforced in postFixup below,
# against the final, stripped artifact.
{
  lib,
  rustPlatform,
  pkgsStatic,
  binutils,
  sourceRev ? "unknown",
}:

let
  # D7 hard gate for the runs-anywhere connect binary.
  connectMaxBytes = 2 * 1024 * 1024; # 2097152

  mkFleet =
    {
      pname,
      features,
      alias ? null,
      sizeGate ? false,
      platform ? rustPlatform,
    }:
    platform.buildRustPackage {
      inherit pname;
      version = "0.1.0";
      src = ./.;
      cargoLock.lockFile = ./Cargo.lock;

      cargoBuildFlags = [
        "-p"
        "fleet"
      ]
      ++ lib.optionals (features != null) [ "--no-default-features" ]
      ++ lib.optionals (features != null && features != [ ]) [
        "--features"
        (lib.concatStringsSep "," features)
      ];

      env.FLEET_SOURCE_REV = sourceRev;

      # Side-by-side installs (NixOS) keep the hyphenated name as a
      # link to the same binary; argv0 narrows the surface.
      postInstall = lib.optionalString (alias != null) ''
        ln -s fleet $out/bin/${alias}
      '';

      # The nixpkgs fixup only strips debug symbols (-S), leaving
      # the symbol table in the shipped binary, so strip fully and
      # explicitly: the D7 contract is about the shipped artifact.
      # (binutils by path: the pkgsStatic stdenv has no bare `strip`
      # on PATH, only the target-prefixed one.)
      postFixup = ''
        ${binutils}/bin/strip $out/bin/fleet
      ''
      + lib.optionalString sizeGate ''
        size=$(stat -c %s $out/bin/fleet)
        echo "${pname}: bin/fleet is $size bytes (D7 gate: <= ${toString connectMaxBytes})"
        if [ "$size" -gt ${toString connectMaxBytes} ]; then
          echo "error: ${pname} binary exceeds the 2 MiB connect size gate (D7)" >&2
          exit 1
        fi
      '';

      meta = {
        description = "Fleet multicall binary (${pname} variant): one binary for the queue, proxy, secrets, search and dispatch domains";
        license = with lib.licenses; [
          mit
          asl20
        ];
        mainProgram = "fleet";
      };
    };
in
{
  # Full profile: every domain compiled in.
  fleet = mkFleet {
    pname = "fleet";
    features = null; # workspace default features = full profile
  };

  # Serve profile: the broker-side domains, for servers.
  fleet-serve = mkFleet {
    pname = "fleet-serve";
    features = [ "serve" ];
    alias = "fleet-serve";
  };

  # Connect profile: proxy-connect only, dynamically linked.
  fleet-connect = mkFleet {
    pname = "fleet-connect";
    features = [ ]; # --no-default-features, nothing optional on
    alias = "fleet-connect";
    sizeGate = true;
  };

  # Connect profile, musl static: runs anywhere, no libc coupling.
  fleet-connect-static = mkFleet {
    pname = "fleet-connect-static";
    features = [ ];
    alias = "fleet-connect";
    sizeGate = true;
    platform = pkgsStatic.rustPlatform;
  };
}
