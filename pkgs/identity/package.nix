{
  lib,
  buildGoModule,
  buildTag ? "sqlite",
  # TEMPORARY (xaltha/13911): lib.fakeHash forces nix to print the true
  # go-modules hash for the post-dependabot go.mod/go.sum; replaced with
  # the computed hash before this branch lands.
  vendorHash ? lib.fakeHash,
}:
buildGoModule {
  pname = "charm-${buildTag}";
  version = "0.1.0";

  src = lib.cleanSourceWith {
    src = ./.;
    filter =
      path: type:
      let
        base = baseNameOf path;
      in
      !(base == "node_modules" || base == "result" || lib.hasPrefix "result-" base);
  };

  inherit vendorHash;
  subPackages = [ "." ];
  tags = [ buildTag ];
  env.CGO_ENABLED = "0";
  ldflags = [
    "-s"
    "-w"
  ];
  doCheck = false;

  postInstall = ''
    if [ -x "$out/bin/identity" ]; then
      mv "$out/bin/identity" "$out/bin/charm"
    elif [ ! -x "$out/bin/charm" ]; then
      echo "expected the identity main package to install an executable" >&2
      ls -la "$out/bin" >&2
      exit 1
    fi
  '';

  meta = {
    description = "Charm identity server (sqlite build)";
    homepage = "https://github.com/developing-today/code/tree/main/pkgs/identity";
    license = lib.licenses.mit;
    mainProgram = "charm";
    platforms = lib.platforms.unix;
  };
}
