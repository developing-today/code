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
#
# Dependency compilation is split from the package build when a
# craneLib is supplied (the root flake's make-fleet passes one):
# crane's buildDepsOnly compiles every registry dependency as its
# OWN derivation keyed on the workspace manifests + Cargo.lock, so
# a fleet source change rebuilds just the workspace crates instead
# of the whole dependency tree — previously every variant below
# recompiled the same dependency set from scratch on any change
# (code#8392). Callers that do not pass craneLib keep the
# monolithic buildRustPackage build. The two musl static variants
# stay monolithic too: under pkgsStatic, crane's cargo environment
# applies crt-static to host build scripts as well (a libc build
# script fails to link against glibc's dynamic objects) — the
# statics need crane's cross-toolchain treatment (build-platform
# toolchain with a musl target plus per-target CC wiring for the
# -sys crates) before they can split. fleet-brokers stays on
# buildRustPackage either way: it is the queue pilot's rollback
# inventory under historical names (it retires at the epic's
# close), and its build/test flag split (--bins to build, lib
# tests to check) does not map onto crane's single cargoExtraArgs
# without changing which tests run.
{
  lib,
  rustPlatform,
  pkgsStatic,
  binutils,
  callPackage,
  craneLib ? null,
  sourceRev ? "unknown",
}:

let
  # ONNX Runtime, pinned to the latest upstream release through the
  # atlas-search package's vendored copy (../atlas-search/onnxruntime):
  # the search serve role's baked library must be the release the
  # corpus and the Gate A fixtures are certified under (code#6765).
  onnxruntime = callPackage ../atlas-search/onnxruntime { };

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
      # atlas-core include_str!s the stage DDL from schema/ (stage.rs);
      # without it the fleet build fails on the missing .sql files.
      ../atlas-search/schema
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
    let
      cargoFlags =
        [
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
      envAttrs = {
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

      metaAttrs = {
        description = "Fleet multicall binary (${pname} variant): one binary for the queue, proxy, secrets, search and dispatch domains";
        license = with lib.licenses; [
          mit
          asl20
        ];
        mainProgram = "fleet";
      };

      craneLibFor = if static then null else craneLib;
    in
    if craneLibFor != null then
      let
        commonArgs = {
          inherit pname;
          version = "0.1.0";
          src = fleetSrc;
          sourceRoot = "${fleetSrc.name}/fleet";
          # The workspace root is the fleet/ subdir of the source,
          # and so is its lockfile; crane vendors from the lockfile
          # path it is given, not from the source root.
          cargoLock = ./Cargo.lock;
          strictDeps = true;
          cargoExtraArgs = "--locked " + lib.concatStringsSep " " cargoFlags;
          env = envAttrs;
        };

        # The dependency layer for this variant's feature set.
        # (crane's dummy source keeps the real source's name, so
        # both layers unpack under the same sourceRoot.)
        cargoArtifacts = craneLibFor.buildDepsOnly commonArgs;
      in
      craneLibFor.buildPackage (
        commonArgs
        // {
          inherit cargoArtifacts;

          # Preserve the buildRustPackage behaviour: the workspace
          # test suite runs in the check phase (it did under
          # buildRustPackage — the store's build logs show a
          # checkPhase with passing test results). The old check
          # phase ran flagless (the whole workspace, default
          # features), while crane's default check appends
          # cargoExtraArgs — which here carries "-p fleet" and
          # would narrow the run to the fleet crate's 27 tests.
          # Override the check command to keep the workspace-wide
          # run; the build keeps the -p fleet scope via
          # cargoExtraArgs.
          doCheck = true;
          checkPhaseCargoCommand = "cargoWithProfile test --locked";

          inherit postInstall postFixup;
          meta = metaAttrs;
        }
      )
    else
      platform.buildRustPackage {
        inherit pname;
        version = "0.1.0";
        src = fleetSrc;
        sourceRoot = "${fleetSrc.name}/fleet";
        cargoLock.lockFile = ./Cargo.lock;

        cargoBuildFlags = cargoFlags;

        env = envAttrs;

        inherit postInstall postFixup;

        meta = metaAttrs;
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
