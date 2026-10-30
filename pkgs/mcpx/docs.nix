# The mcpx book as a flake package: .#mcpx-docs (issue #1939).
#
# runCommand-style on purpose: the book is a pure function of the docs
# tree and the pinned mdbook binary -- no network, no deploy wiring
# (deployment is a sibling issue), nothing but mdbook over SUMMARY.md.
{ lib, runCommand, mdbook }:

runCommand "mcpx-docs"
{
  nativeBuildInputs = [ mdbook ];
  meta = {
    description = "The mcpx book (mdBook) built from pkgs/mcpx/docs";
    homepage = "https://github.com/developing-today/code/tree/main/pkgs/mcpx/docs";
    license = lib.licenses.mit;
  };
}
  ''
    cp -r ${./docs} book
    chmod -R u+w book
    mdbook build --dest-dir "$out" book
  ''
