# The mcpx book (#1939, Approach C): render pkgs/mcpx/docs with mdBook
# into static HTML, sandboxed, with the flake-pinned mdbook from
# nixpkgs and no network access. Exposed as the .#mcpx-docs flake
# output, so CI builds it through the ordinary flake-check path like
# every other output.
{
  lib,
  runCommand,
  mdbook,
}:
let
  # Only the book inputs enter the store: book.toml plus the docs tree.
  # The Go sources next to them are not book content and stay out.
  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./book.toml
      ./docs
    ];
  };
in
runCommand "mcpx-docs"
  {
    nativeBuildInputs = [ mdbook ];
    meta = {
      description = "mcpx documentation book (mdBook rendering of pkgs/mcpx/docs)";
      homepage = "https://github.com/developing-today/code";
      license = with lib.licenses; [
        mit
        asl20
      ];
    };
  }
  ''
    cp -r ${src} work
    chmod -R u+w work
    cd work
    mdbook build --dest-dir $out
  ''
