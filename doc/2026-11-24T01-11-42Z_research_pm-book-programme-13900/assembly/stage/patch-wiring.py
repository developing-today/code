#!/usr/bin/env python3
"""PM-ASSEMBLY wiring patches, run inside ~/wt-pm-book. Every anchor asserted."""

import pathlib

root = pathlib.Path.cwd()

# --- lib/default.nix ---
p = root / "lib/default.nix"
t = p.read_text()
old = "      idDocs = pkgs.callPackage ../pkgs/id/docs.nix { };\n"
assert t.count(old) == 1
t = t.replace(old, old + "      pmDocs = pkgs.callPackage ../pm-stack/docs.nix { };\n")
old2 = "        id-docs = idDocs;\n"
assert t.count(old2) == 1
t = t.replace(old2, old2 + "        pm-docs = pmDocs;\n")
p.write_text(t)
print("lib/default.nix patched")

# --- .github/workflows/mcpx-docs.yml : add pm steps to the deploy job ---
p = root / ".github/workflows/mcpx-docs.yml"
t = p.read_text()
anchor = """      - name: Stage id book into site/id
        run: |
          rm -rf site/id
          # site/ is a copy of a Nix store path: its directories are
          # read-only, so make the root writable before adding id/.
          chmod u+w site
          cp -rL result-id-docs site/id
          test -f site/id/index.html
"""
assert t.count(anchor) == 1, "id stage anchor not found exactly once"
pm_steps = (
    anchor
    + """
      # The Project Management book (#13900, pm-stack/book) shares
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
)
t = t.replace(anchor, pm_steps)
p.write_text(t)
print("mcpx-docs.yml patched")

import yaml

d = yaml.safe_load(p.read_text())
steps = d["jobs"]["deploy"]["steps"]
names = [s.get("name", "") for s in steps]
assert any("PM book" in n for n in names), names
print("YAML parses; deploy steps:", len(steps))
print([n for n in names if "PM" in n or "pm" in n])
