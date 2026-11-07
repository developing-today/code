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
  onnxruntime,
  sourceRev ? "unknown",
}:

let
  # The search serve role (#5604) consumes atlas-core / atlas-embed /
  # atlas-proto by relative path (../atlas-search/crates/*, untouched),
  # so the build source is the pkgs/ tree holding both workspaces and
  # cargo runs from pkgs/fleet inside it. The fleet Cargo.lock covers
  # the atlas crates' external dependencies.
  fleetSrc = lib.fileset.toSource {
    root = ./..;
    fileset = lib.fileset.unions [
      ./.
      ../atlas-search/Cargo.toml
      ../atlas-search/crates
    ];
  };

  # D7 hard gate for the runs-anywhere connect binary.
  connectMaxBytes = 2 * 1024 * 1024; # 2097152

  mkFleet =
    {
      pname,
      features,
      alias ? null,
      sizeGate ? false,
      platform ? rustPlatform,
      static ? false,
    }:
    platform.buildRustPackage {
      inherit pname;
      version = "0.1.0";
      src = fleetSrc;
      sourceRoot = "${fleetSrc.name}/fleet";
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

      # ONNX Runtime is dynamically loaded (atlas-embed's ort
      # load-dynamic) and never downloaded; the search serve role
      # resolves it from this closure path (the atlas-search package's
      # own ATLAS_SEARCH_ORT_LIB_DEFAULT mechanism, ported). Compiled
      # in only where search-serve is. Not set for the connect
      # variants (no search-serve) nor the musl static ones (a glibc
      # libonnxruntime cannot be dlopen'd from a static binary; the
      # role there needs ATLAS_ORT_LIB).
      env = {
        FLEET_SOURCE_REV = sourceRev;
      }
      // lib.optionalAttrs (features != [ ] && !static) {
        ATLAS_SEARCH_ORT_LIB_DEFAULT = "${onnxruntime}/lib/libonnxruntime.so";
      };

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

  # The standalone broker artifacts (queue domain, step 2 #5430):
  # repo-queue / queue-broker / secrets-broker built from the same
  # workspace under their historical names — the migration's
  # rollback inventory (design note §7). While the queue pilot's
  # flips proceed, a unit or symlink can always be re-pointed at
  # one of these; they retire only at the epic's close, not before.
  fleet-brokers = rustPlatform.buildRustPackage {
    pname = "fleet-brokers";
    version = "0.1.0";
    src = fleetSrc;
    sourceRoot = "${fleetSrc.name}/fleet";
    cargoLock.lockFile = ./Cargo.lock;

    cargoBuildFlags = [
      "-p"
      "fleet-queue"
      "--features"
      "client,queue-broker,secrets-broker"
      "--bins"
    ];
    cargoTestFlags = [
      "-p"
      "fleet-queue"
      "--features"
      "client,queue-broker,secrets-broker"
    ];

    # Same explicit full strip as mkFleet (nixpkgs fixup leaves the
    # symbol table in).
    postFixup = ''
      ${binutils}/bin/strip $out/bin/repo-queue $out/bin/queue-broker $out/bin/secrets-broker
    '';

    meta = {
      description = "Fleet queue-domain standalone artifacts (repo-queue, queue-broker, secrets-broker): the queue pilot's rollback inventory";
      license = with lib.licenses; [
        mit
        asl20
      ];
    };
  };

  # Full profile, musl static: the same capability set as `fleet`,
  # linked for foreign soil (refinement B: linkage is a per-context
  # choice). This is the artifact non-nix hosts install — the queue
  # pilot's sandbox `repo-queue` symlink flip (#5430) points at a
  # copy of it, because a glibc-dynamic laptop build names store
  # paths the sandbox does not have.
  fleet-static = mkFleet {
    pname = "fleet-static";
    features = null; # workspace default features = full profile
    platform = pkgsStatic.rustPlatform;
    static = true;
  };

  # Connect profile, musl static: runs anywhere, no libc coupling.
  fleet-connect-static = mkFleet {
    pname = "fleet-connect-static";
    features = [ ];
    alias = "fleet-connect";
    sizeGate = true;
    platform = pkgsStatic.rustPlatform;
    static = true;
  };
}
