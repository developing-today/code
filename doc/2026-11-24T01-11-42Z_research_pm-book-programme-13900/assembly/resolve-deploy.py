#!/usr/bin/env python3
"""Resolve the mcpx-docs.yml merge conflict: keep origin/main's file
(Code Book + Lab Book steps) and insert the PM steps before configure-pages."""
import pathlib, subprocess

root = pathlib.Path.cwd()
p = root / ".github/workflows/mcpx-docs.yml"
main_version = subprocess.run(
    ["git", "show", "origin/main:.github/workflows/mcpx-docs.yml"],
    capture_output=True, text=True, check=True).stdout
assert "code-docs" in main_version and "lab-docs" in main_version
assert "pm-docs" not in main_version

anchor = "      - uses: actions/configure-pages@v6\n"
assert main_version.count(anchor) == 1
pm_steps = """      # The Project Management book (#13900, pm-stack/book) shares
      # this repository's single Pages site the same way: built with
      # its own siteUrl and staged under pm/ here, in the same deploy
      # — a separate pm-only deploy would wipe the mcpx, fleet, atlas
      # and id books. Its build-only workflow is pm-docs.yml; see
      # the book's "Deploying this book" chapter
      # (pm-stack/book/operations/deploying-this-book.md).
      - name: Nix build (PM book, siteUrl /code/pm/)
        run: |
          # Intentional: the single-quoted string in this block is a Nix expression
          # evaluated by nix, not the shell -- ${builtins.currentSystem} in it is
          # Nix's, and letting the shell expand it would break the build.
          # shellcheck disable=SC2016
          nix build --impure --print-build-logs --expr \\
            '(builtins.getFlake (toString ./.)).packages.${builtins.currentSystem}.pm-docs.override { siteUrl = "/code/pm/"; }' \\
            --out-link result-pm-docs

      - name: Stage PM book into site/pm
        run: |
          rm -rf site/pm
          # site/ is a copy of a Nix store path: its directories are
          # read-only, so make the root writable before adding pm/.
          chmod u+w site
          cp -rL result-pm-docs site/pm
          test -f site/pm/index.html

"""
resolved = main_version.replace(anchor, pm_steps + anchor)
assert "<<<<<<<" not in resolved and ">>>>>>>" not in resolved
p.write_text(resolved)
print("resolved; pm steps inserted before configure-pages")
