# Read-only eval of the id package's nativeBuildInputs names from the copied pkgs/id flake (no instantiation, no build).
let
  flake = builtins.getFlake (builtins.getEnv "PKGID_FLAKE");
  sys = "x86_64-linux";
  pk = flake.packages.${sys} or { };
  names = drv: map (d: if d ? name then d.name else "?") (drv.nativeBuildInputs or [ ]);
in
builtins.concatStringsSep "\n" (
  [ "packages=${builtins.concatStringsSep "," (builtins.attrNames pk)}" ]
  ++ map (k: "${k}.nativeBuildInputs=[${builtins.concatStringsSep ", " (names pk.${k})}]")
       (builtins.filter (k: builtins.elem k [ "id-lib" "id-web" "default" ]) (builtins.attrNames pk))
  ++ [ "" ]
)
