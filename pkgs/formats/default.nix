{ pkgs }:
{
  gzipJson =
    _:
    {
      generate =
        name: value:
        pkgs.callPackage (
          { runCommand, gzip }:
          runCommand name
            {
              nativeBuildInputs = [ gzip ];
              value = builtins.toJSON value;
              passAsFile = [ "value" ];
            }
            ''
              gzip "$valuePath" -c > "$out"
            ''
        ) { };

      inherit ((pkgs.formats.json { })) type;
    };
}
