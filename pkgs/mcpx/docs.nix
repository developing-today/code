# The mcpx book (#1939, Approach C): render pkgs/mcpx/docs with mdBook
# into static HTML, sandboxed, with the flake-pinned mdbook from
# nixpkgs and no network access. Exposed as the .#mcpx-docs flake
# output, so CI builds it through the ordinary flake-check path like
# every other output.
#
# The base URL is a derivation argument (#1941): `siteUrl` is baked
# into the rendered book as mdBook's output.html.site-url, so a local
# build, the CI build, and a deployed build differ only in that one
# value. The flake output uses the default "/"; the GitHub Pages
# deploy overrides it with "/code/" (the project site's path — mdBook
# needs the real hosting path or its generated 404 page points assets
# at the domain root), e.g. `mcpxDocs.override { siteUrl = "/code/"; }`.
{
  lib,
  runCommand,
  mdbook,
  buildGo127Module,
  siteUrl ? "/",
}:
let
  # The renderer only uses the Go standard library. Build the small tool
  # independently of mcpx's runtime dependencies so the book build stays
  # offline and does not pull the full application into its closure.
  docsHistory = buildGo127Module {
    pname = "docs-history";
    version = "0.0.0";
    src = lib.fileset.toSource {
      root = ./.;
      fileset = lib.fileset.unions [
        ./go.mod
        ./go.sum
        ./tools/docs-history
      ];
    };
    vendorHash = null;
    # This module is a stdlib-only subset of a larger repository whose
    # vendor directory is intentionally not in src.
    proxyVendor = true;
    subPackages = [ "tools/docs-history" ];
    env.CGO_ENABLED = "0";
    # This stdlib-only package must not fetch the unrelated mcpx dependencies.
    env.GOPROXY = "off";
  };

  # The agent-twin emitter (#1947): same stdlib-only pattern as
  # docsHistory above. Runs after mdbook build to write the per-page
  # .html.md twins + llms files into the site tree, then gates the
  # build on every page having a current twin.
  docsTwins = buildGo127Module {
    pname = "docs-twins";
    version = "0.0.0";
    src = lib.fileset.toSource {
      root = ./.;
      fileset = lib.fileset.unions [
        ./go.mod
        ./go.sum
        ./tools/docs-twins
      ];
    };
    vendorHash = null;
    proxyVendor = true;
    subPackages = [ "tools/docs-twins" ];
    env.CGO_ENABLED = "0";
    env.GOPROXY = "off";
  };

  # Only the book inputs enter the store: book.toml plus the docs tree.
  # The Go sources next to them are not book content and stay out.
  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./book.toml
      ./docs
      # Hand-written agent twins (overrides) for docs-twins emit.
      ./agents-docs
    ];
  };
in
runCommand "mcpx-docs"
  {
    nativeBuildInputs = [
      mdbook
      docsHistory
      docsTwins
    ];
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
    # Agent twins (#1947): emit the per-page .html.md twins, llms.txt
    # and llms-full.txt into the built site (twin content is the
    # docs-history agent render of each page; the home page comes from
    # agents-docs), inject the discovery <link> tags, then gate: the
    # build fails if any page lacks a current twin.
    docs-twins emit --docs docs --agents agents-docs --out $out --history-bin docs-history
    docs-twins check --docs docs --agents agents-docs --out $out --history-bin docs-history
  ''
