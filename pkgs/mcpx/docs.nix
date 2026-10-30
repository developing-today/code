# The mcpx book (#1939, Approach C): render pkgs/mcpx/docs with mdBook
# into static HTML, sandboxed, with the flake-pinned mdbook from
# nixpkgs and no network access. Exposed as the .#mcpx-docs flake
# output, so CI builds it through the ordinary flake-check path like
# every other output.
#
# The base URL is a derivation argument (#1941): `siteUrl` is baked
# into the rendered book as mdBook's output.html.site-url, so a local
# build, the CI build, and a future custom-domain build differ only in
# that one value. The flake output uses the default "/" (the GitHub
# Pages Actions deploy root); other deployments override the argument,
# e.g. `mcpxDocs.override { siteUrl = "/code/"; }`.
{
  lib,
  runCommand,
  mdbook,
  siteUrl ? "/",
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
    # Links that escape the book source tree would 404 in the rendered
    # book: from docs/, ../AGENTS.md means pkgs/mcpx/AGENTS.md, which is
    # not part of the artifact. Rewrite those links - in the copied
    # pages only, the sources keep their relative links for GitHub - to
    # the file's repository URL, at the depth where the link escapes:
    # ../ from a top-level page, ../../ from a page one directory down.
    # Links that stay inside docs/ are left for mdBook to resolve.
    repoURL="https://github.com/developing-today/code/blob/main/pkgs/mcpx/"
    find docs -maxdepth 1 -name '*.md' -exec sed -i \
      "s|](\.\./|](''${repoURL}|g" {} +
    find docs -mindepth 2 -maxdepth 2 -name '*.md' -exec sed -i \
      "s|](\.\./\.\./|](''${repoURL}|g" {} +
    # Bake the base URL into the book (see the siteUrl argument above).
    export MDBOOK_OUTPUT__HTML__SITE_URL="${siteUrl}"
    mdbook build --dest-dir $out
  ''
