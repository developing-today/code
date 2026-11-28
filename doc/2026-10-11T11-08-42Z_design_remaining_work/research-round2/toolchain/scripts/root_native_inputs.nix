# Read-only eval of the root flake's id package nativeBuildInputs names, from a copy of the integrated repo (no build, no instantiation).
let
  flake = builtins.getFlake (builtins.getEnv "ROOT_FLAKE");
  sys = "x86_64-linux";
  pk = flake.packages.${sys} or { };
  names = drv: map (d: if d ? name then d.name else "?") (drv.nativeBuildInputs or [ ]);
  idNames = builtins.filter (k: builtins.match ".*id.*" k != null) (builtins.attrNames pk);
in
builtins.concatStringsSep "\n" (
  [ "root packages with id in name=${builtins.concatStringsSep "," idNames}" ]
  ++ map (k: "${k}.nativeBuildInputs=[${builtins.concatStringsSep ", " (names pk.${k})}]") (builtins.filter (k: builtins.elem k [ "id-web" "id-lib" ]) idNames)
  ++ [ "" ]
)
