# mcpx

mcpx is the project's MCP gateway: it puts every MCP (Model Context
Protocol) server behind one local daemon and gives agents a TypeScript
client — code mode — instead of dozens of tool schemas in context. A script
written once runs again at no context cost; a script that earns its place is
published as a named script. One MCP configuration is shared across clients,
can be shared on the tailnet, and carries per-client policy: which servers a
client may see, and the capability ceiling it runs under.

mcpx is the oldest book in the project and the largest: 84 chapters and
about 288,000 words at the 2026-11-23 audit, published at the Pages site
root. Its book predates the house form the later books follow, and the
2026-11-23 run aligned it: a coverage audit against the binary's own dumped
surface, with disagreements filed as #13420 and #13422.

## Current state (2026-11-23)

- The daemon, about 50 commands and 252 settings are live. Census figures in
  programme documents rot fast, and the mcpx audit re-derived them from the
  binary on the day: the settings count differs by whether test fixtures are
  counted, and the book states its counting rule with the number.
- Completion work is merged-list item 25, in flight and close on several
  fronts: the supervisor design (#1384) and supervised stdio by default
  (#9963) have merged PRs with their acceptance check carried by the
  programme tracker; session resume (#1414) has phases 1–2 landed with the
  phase-3 question open; OAuth for remote servers (#1950) landed 2026-11-23
  (PR #12869).
- The settings effect-probe exemption tail is exactly 8 settings: 5 unheld
  and unclaimed, and 3 held under platform execution-risk flags
  (`plugin.instructions` and `plugin.disabledTools` via #8472,
  `protoTasks.cancelGrace` via #8473).
- The Atlas integration is live: the laptop's mcpx carries the `atlas`
  remote server, so agents reach Atlas search through the gateway.

## Where its detail lives

- Book: the [mcpx book](https://developing-today.github.io/code/) — the site
  root; sources at `pkgs/mcpx/docs/`.
- This programme's slice: the mcpx book's [high-level
  roadmap](https://developing-today.github.io/code/program/high-level-roadmap.html)
  carries mcpx's share of the merged list (item 25 in full). The mcpx book
  had no programme-roadmap chapter before this programme; its roadmap lived
  in its trackers, headlined by the super-tracker
  [#1900](https://github.com/developing-today/code/issues/1900).
- Programme tracker: [#13264](https://github.com/developing-today/code/issues/13264).
- Merged-list items: 25 is mcpx-primary; mcpx also carries cross-links to
  items 12, 14, 22, 27, 28, 29 and 30.
