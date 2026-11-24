# The Project Management book (#13900): render pm-stack/book with
# mdBook into static HTML, sandboxed, with the flake-pinned mdbook
# from nixpkgs and no network access. Exposed as the .#pm-docs flake
# output, so CI builds it through the ordinary flake-check path like
# every other output. Mirrors pkgs/fleet/docs.nix (#13165), which
# mirrors pkgs/mcpx/docs.nix (#1939, Approach C) minus the
# mcpx-specific Go preprocessors, which the PM book does not use.
#
# The base URL is a derivation argument (the #1941 pattern): `siteUrl`
# is baked into the rendered book as mdBook's output.html.site-url, so
# a local build, the CI build, and a deployed build differ only in
# that one value. The flake output uses the default "/"; the GitHub
# Pages deploy overrides it with "/code/pm/" (the PM book's path
# inside the repository's project site — mdBook needs the real
# hosting path or its generated 404 page points assets at the domain
# root), e.g. `pmDocs.override { siteUrl = "/code/pm/"; }`.
{
  lib,
  runCommand,
  mdbook,
  siteUrl ? "/",
}:
let
  # Only the book inputs enter the store: book.toml plus the book
  # tree. The rest of the pm-stack tree is not book content and stays
  # out.
  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./book.toml
      ./book
    ];
  };
in
runCommand "pm-docs"
  {
    nativeBuildInputs = [
      mdbook
    ];
    meta = {
      description = "Project Management documentation book (mdBook rendering of pm-stack/book)";
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
    # book: from book/, ../README.md means pm-stack/README.md, which
    # is not part of the artifact. Rewrite those links — in the copied
    # pages only, the sources keep their relative links for GitHub —
    # to the file's repository URL, at the depth where the link
    # escapes: ../ from a top-level page, ../../ from a page one
    # directory down. Links that stay inside book/ are left for
    # mdBook to resolve. (Same treatment as pkgs/fleet/docs.nix.)
    repoURL="https://github.com/developing-today/code/blob/main/pm-stack/"
    find book -maxdepth 1 -name '*.md' -exec sed -i \
      "s|](\.\./|](''${repoURL}|g" {} +
    find book -mindepth 2 -maxdepth 2 -name '*.md' -exec sed -i \
      "s|](\.\./\.\./|](''${repoURL}|g" {} +
    # Bake the base URL into the book (see the siteUrl argument above).
    export MDBOOK_OUTPUT__HTML__SITE_URL="${siteUrl}"
    mdbook build --dest-dir $out
  ''
