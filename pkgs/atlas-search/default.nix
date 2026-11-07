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
{
  lib,
  rustPlatform,
  onnxruntime,
  python3,
}:

rustPlatform.buildRustPackage {
  pname = "atlas-search";
  version = "0.1.0";
  src = ./.;

  cargoLock.lockFile = ./Cargo.lock;

  env.ATLAS_SEARCH_ORT_LIB_DEFAULT = "${onnxruntime}/lib/libonnxruntime.so";

  nativeInstallCheckInputs = [ python3 ];
  doInstallCheck = true;
  # The smoke: fixture FTS-only + embedded queries byte-compared
  # with the recorded Python envelopes, plus the closure-ONNX
  # load probe (an empty model dir must fail at the tokenizer,
  # after runtime init — never at runtime resolution).
  installCheckPhase = ''
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
}
