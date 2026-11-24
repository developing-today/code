# idp-provider — minimal OIDC provider (Slice 3 of #13658, implements
# #13409). Follows the repo's small-Rust-package pattern
# (rustPlatform.buildRustPackage with cargoLock, as in the
# fleet-brokers package in pkgs/fleet and the atlas-search monolith).
{ lib, rustPlatform }:

rustPlatform.buildRustPackage {
  pname = "idp-provider";
  version = "0.1.0";
  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;

  meta = {
    description = "Minimal OIDC provider: discovery, JWKS, authorize/token/userinfo (code + PKCE) signing with the IdP PKI key";
    license = with lib.licenses; [
      mit
      asl20
    ];
    mainProgram = "idp-provider";
  };
}
