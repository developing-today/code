{ pkgs }:

pkgs.stdenv.mkDerivation {
  pname = "idp-pki";
  version = "0.1.0";
  src = ./.;

  nativeBuildInputs = [ pkgs.makeWrapper ];

  installPhase = ''
    runHook preInstall
    install -Dm755 idp-ca.sh $out/bin/idp-ca
    wrapProgram $out/bin/idp-ca \
      --prefix PATH : ${
        pkgs.lib.makeBinPath [
          pkgs.openssl
          pkgs.coreutils
        ]
      }
    install -Dm644 README.md $out/share/doc/idp-pki/README.md
    install -Dm644 trust/root-ca.crt.pem $out/share/idp-pki/trust/root-ca.crt.pem
    install -Dm644 trust/issuing-ca.crt.pem $out/share/idp-pki/trust/issuing-ca.crt.pem
    install -Dm644 trust/chain.pem $out/share/idp-pki/trust/chain.pem
    runHook postInstall
  '';

  meta = {
    description = "Small openssl-backed CA for the self-hosted identity provider program";
    mainProgram = "idp-ca";
  };
}
