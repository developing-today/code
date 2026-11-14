# atlas-search — the Atlas access layer: hybrid FTS+vector search
# CLI + multi-client service (developing-today/code#5350, step 5:
# #5355). Build it through the package flake:
#
#   nix build path:./pkgs/atlas-search
#
# or via home/common/default.nix, which callPackages this file
# with the same pinned toolchain (the pkgs/id pattern).
#
# ONNX Runtime is dynamically loaded (the ort crate's
# load-dynamic feature) and is NEVER downloaded at build or run
# time (design section 4, the loader lesson). The packaged binary
# resolves it from its own closure: ATLAS_SEARCH_ORT_LIB_DEFAULT
# is compiled in below, pointing at the onnxruntime store path —
# main.rs's resolve_ort_lib falls back to it after the env and
# state-dir candidates. Because the path is a store reference,
# onnxruntime — and with it libstdc++.so.6 and libz.so.1 — is
# part of this package's runtime closure (verify:
# `nix-store -qR <out>`), and the installCheck proves the load
# works with no ATLAS_ORT_LIB set (fixtures/smoke_check.py).
#
# Dependency compilation is split from the package build when a
# craneLib is supplied (the package flake passes one): crane's
# buildDepsOnly compiles every registry dependency as its OWN
# derivation keyed only on Cargo.toml/Cargo.lock, so a change to
# workspace sources rebuilds just the workspace crates instead of
# the whole dependency tree (code#8392). Callers that do not pass
# craneLib (home/common) keep the monolithic buildRustPackage
# build below — same sources, same checks, no split.
{
  lib,
  rustPlatform,
  callPackage,
  python3,
  craneLib ? null,
}:

let
  # ONNX Runtime is pinned to the latest upstream release, vendored
  # in-tree (./onnxruntime) — never the ambient nixpkgs version.
  # The corpus and the Gate A fixtures are embedded/certified under
  # that release; the library baked in below must be the same one
  # (code#6765, Drew's ruling 2026-11-08).
  onnxruntime = callPackage ./onnxruntime { };

  ortLibEnv = {
    ATLAS_SEARCH_ORT_LIB_DEFAULT = "${onnxruntime}/lib/libonnxruntime.so";
  };

  installCheck = ''
    runHook preInstallCheck
    python3 ${./fixtures}/smoke_check.py $out/bin/atlas-search
    runHook postInstallCheck
  '';

  meta = {
    description = "Atlas corpus search: hybrid FTS+vector CLI and multi-client service (Snowflake arctic-embed-m-v2.0)";
    license = with lib.licenses; [
      mit
      asl20
    ];
    mainProgram = "atlas-search";
  };
in
if craneLib != null then
  let
    # Two source tiers. The dependency layer sees only what can
    # change dependency compilation: the manifests, the lockfile,
    # the crate sources and schema/ (atlas-core include_str!s the
    # stage DDL from schema/). The package layer additionally sees
    # fixtures/: the test suite reads fixtures by relative path
    # (atlas-core's eval tests open ../../fixtures/eval-smoke.json),
    # so the check phase needs them in the tree — but a fixture edit
    # must not rebuild the dependency layer, so they stay out of
    # the deps tier. eval/, worker/ and the in-tree target/ are in
    # neither (the installCheck references ./fixtures as its own
    # store path, above).
    depsSrc = lib.fileset.toSource {
      root = ./.;
      fileset = lib.fileset.unions [
        ./Cargo.toml
        ./Cargo.lock
        ./crates
        ./schema
      ];
    };

    packageSrc = lib.fileset.toSource {
      root = ./.;
      fileset = lib.fileset.unions [
        ./Cargo.toml
        ./Cargo.lock
        ./crates
        ./schema
        ./fixtures
      ];
    };

    commonArgs = {
      strictDeps = true;
      pname = "atlas-search";
      version = "0.1.0";
      env = ortLibEnv;
    };

    # The dependency layer: every registry crate compiled once,
    # keyed on the deps tier above only. A workspace-source or
    # fixture change leaves this derivation — and its store path —
    # untouched.
    cargoArtifacts = craneLib.buildDepsOnly (commonArgs // { src = depsSrc; });
  in
  craneLib.buildPackage (
    commonArgs
    // {
      src = packageSrc;
      inherit cargoArtifacts meta;

      # Preserve the buildRustPackage behaviour this package had:
      # the test suite runs in the check phase, and the smoke
      # installCheck runs against the installed tree.
      doCheck = true;

      nativeInstallCheckInputs = [ python3 ];
      doInstallCheck = true;
      installCheckPhase = installCheck;
    }
  )
else
  rustPlatform.buildRustPackage {
    pname = "atlas-search";
    version = "0.1.0";
    src = ./.;

    cargoLock.lockFile = ./Cargo.lock;

    env = ortLibEnv;

    nativeInstallCheckInputs = [ python3 ];
    doInstallCheck = true;
    # The smoke: fixture FTS-only + embedded queries byte-compared
    # with the recorded Python envelopes, plus the closure-ONNX
    # load probe (an empty model dir must fail at the tokenizer,
    # after runtime init — never at runtime resolution).
    installCheckPhase = installCheck;

    inherit meta;
  }
