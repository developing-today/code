# Catalogue

The catalogue chapter: things outside mcpx itself — MCP servers, registries,
resources, URLs and projects — and why each one is here.

- **Source** this directory, one page per section: `servers.md`,
  `registries.md`, `resources.md`, `urls.md`, `projects.md`.
- **Generated or written: C.** The list is hand-written — it says *which*
  servers, registries, resources, URLs and projects are in the catalogue and
  *why*; the facts per server are captured into a committed snapshot by a
  refresh script. Nothing at docs build time starts a server or calls the
  network.
- **First entries** are the seven servers configured on this machine —
  `codebase-memory`, `codedb`, `fff`, `fff-nix`, `fff-worktree`,
  `chrome-devtools` (the `session`-mode case) and `context7` — plus the
  official registry `registry.modelcontextprotocol.io`, the recorded
  rejection (HAPI), and the projects list from #1867.

The structure is recorded in `docs/compendium.md` ("The catalogue chapter");
the section pages are seeded by the follow-up task. Approach A (hand-written
entries only) and B (mirror `mcpx registry search`) stay rejected for the
reasons on #1867.
