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
  openssl,
  binutils,
  python3,
  bash,
  util-linux,
  callPackage,
  craneLib ? null,
  doCheck ? true,
  sourceRev ? "unknown",
  # The derived per-app version (#8566), computed by the release
  # tooling where the git history exists (the nix build source has
  # no .git). A release build passes the clean tag version here
  # (e.g. "0.1.0" at fleet/v0.1.0); left null, the package derives
  # an honest prerelease from sourceRev below.
  derivedVersion ? null,
}:

let
  # ONNX Runtime, pinned to the latest upstream release through the
  # atlas-search package's vendored copy (../atlas-search/onnxruntime):
  # the search serve role's baked library must be the release the
  # corpus and the Gate A fixtures are certified under (code#6765).
  onnxruntime = callPackage ../atlas-search/onnxruntime { };

  # Tools the workspace test suite needs in the check phase: Python's
  # sqlite3 for the concurrency regressions, and bash plus util-linux
  # (setsid) for the fleet-agent shell tests.
  checkTools = [
    python3
    bash
    util-linux
  ];

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

  # OpenSSL the openssl-sys build script can link (code#16152):
  # fleet-cert's PKCS#12 boundary made `openssl` a mandatory
  # workspace dependency, so every leg that compiles the workspace
  # (build and check phases alike) needs one. Explicit
  # DIR/LIB_DIR/INCLUDE_DIR so no pkg-config program is required in
  # any build env; the musl legs take the static-only archives with
  # OPENSSL_STATIC selecting them.
  nativeOpensslEnv = {
    OPENSSL_DIR = "${openssl.dev}";
    OPENSSL_LIB_DIR = "${openssl.out}/lib";
    OPENSSL_INCLUDE_DIR = "${openssl.dev}/include";
  };
  staticOpensslEnv = {
    OPENSSL_STATIC = "1";
    OPENSSL_DIR = "${pkgsStatic.openssl.dev}";
    OPENSSL_LIB_DIR = "${pkgsStatic.openssl.out}/lib";
    OPENSSL_INCLUDE_DIR = "${pkgsStatic.openssl.dev}/include";
  };

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
      cargoFlags = [
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
        # #13617: the gh_metadata conformance test reads the
        # pm-metadata v1 fixture set from doc/ at the repository root,
        # which the fleetSrc fileset does not carry; point the test at
        # this store copy of the fixture instead.
        PM_METADATA_V1_FIXTURE = "${
          ../../doc/2026-11-21T17-56-24Z_design_atlas_metadata_schema_v1/fixtures/pm-metadata-v1.json
        }";
        # #8566: the binary's Derived version channel. An explicit
        # derivedVersion wins; otherwise a real sourceRev yields a
        # unique prerelease form (0.1.0-dev+g<rev7>) instead of the
        # colliding bare 0.1.0-dev every packaged build would
        # otherwise report. "unknown"/"dirty" pass nothing and the
        # build script's own fallback applies.
        FLEET_DERIVED_VERSION =
          if derivedVersion != null then
            derivedVersion
          else if sourceRev != "unknown" && sourceRev != "dirty" then
            "0.1.0-dev+g${lib.substring 0 7 sourceRev}"
          else
            "0.1.0-dev";
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
          # buildDepsOnly also builds test targets unless this is forwarded.
          # Runtime NixOS packages skip those; checked package outputs keep them.
          inherit doCheck;
          # The native OpenSSL for openssl-sys (code#16152); the
          # crane legs compile the workspace's dependency closure for
          # glibc, which includes fleet-cert's mandatory `openssl`.
          buildInputs = [ openssl ];
          env = envAttrs // nativeOpensslEnv;
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
          # SQLite concurrency regressions use Python's stdlib sqlite3.
          # The fleet-agent shell tests run bash scripts and the
          # target-lease plan's setsid; the build sandbox has neither
          # /bin/bash nor util-linux on PATH unless listed here.
          nativeCheckInputs = checkTools;
          inherit doCheck;
          checkPhaseCargoCommand = "cargoWithProfile test --locked";

          # The atlas-search server tests (#9973) write trace and
          # feedback records under $HOME/.local/state/atlas-search;
          # the build sandbox's HOME (/homeless-shelter) is not
          # writable, which failed every variant's check phase in
          # any sandboxed build. Point HOME at a build temp dir for
          # the check phase (#6400).
          preCheck = ''
            export HOME="$TMPDIR/fleet-check-home"
            mkdir -p "$HOME"
          '';

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

        nativeCheckInputs = checkTools;
        cargoBuildFlags = cargoFlags;
        inherit doCheck;

        # Static (musl) builds need an OpenSSL the openssl-sys build
        # script can link: fleet-cert's PKCS#12 boundary made
        # `openssl` a mandatory workspace dependency, and the static
        # variants' workspace-wide check phase compiles it for
        # x86_64-unknown-linux-musl (code#16152). The same holds for
        # the native monolithic path (used when no craneLib is
        # passed), against the glibc OpenSSL.
        nativeBuildInputs = lib.optionals static [ pkgsStatic.openssl ];
        buildInputs = lib.optionals static [ pkgsStatic.openssl ]
          ++ lib.optionals (!static) [ openssl ];

        # Same check-phase HOME fix as the crane path above (#6400).
        preCheck = ''
          export HOME="$TMPDIR/fleet-check-home"
          mkdir -p "$HOME"
        '';

        env = envAttrs // (if static then staticOpensslEnv else nativeOpensslEnv);

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
    nativeCheckInputs = checkTools;
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

    # Native OpenSSL for openssl-sys (code#16152): fleet-queue links
    # fleet-cert, whose PKCS#12 boundary made `openssl` mandatory.
    buildInputs = [ openssl ];
    env = nativeOpensslEnv;

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
