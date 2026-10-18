# Plans

Implementation plans break a roadmap item into steps. Each step is small enough for one PR and says how to tell it is done. Use the [plan template](../templates/plan.md).

New plans go in this folder. Name a plan `<short-name>.md`, or `<UTC datetime>_<short-name>.md` when the order matters. Link each plan from the roadmap row it implements, and keep its Progress section current as PRs merge.

## Plans written before this folder existed

These stay in [`.opencode/plans/`](../../.opencode/plans/). They are not moved, because other documents link to them by path.

| Plan                                                                                             | Covers                                                                                             | State                                                              |
| ------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| [Phase 9 and 10](../../.opencode/plans/phase-9-portable-platform.md)                             | Portable world surfaces, execution profiles, standard libraries, roles, fixtures, scheduled events | Mostly shipped. See the `id` roadmap.                              |
| [Phase 11](../../.opencode/plans/phase-11-foundations.md)                                        | CI trust, cost visibility, module split, journal tooling, cheaper guests                           | In progress. Its status lines lag the merged PRs, tracked in #312. |
| [Phase 12](../../.opencode/plans/phase-12-platform-flexibility.md)                               | World manifest, provider registry, more guest languages, upgrades, scale                           | Not started.                                                       |
| [Cursor enhancements](../../.opencode/plans/2026-03-20_cursor_enhancements.md)                   | Collaborative cursors in the web editor                                                            | Older. State not checked.                                          |
| [Peer discovery](../../.opencode/plans/2026-03-21_peer-discovery.md)                             | Finding peers                                                                                      | Older. State not checked.                                          |
| [Tags v2 on iroh docs](../../.opencode/plans/2026-03-24-tags-v2-iroh-docs.md)                    | Tag storage                                                                                        | Older. State not checked.                                          |
| [Tags v2 polish](../../.opencode/plans/2026-03-24-tags-v2-polish.md)                             | Tag follow-ups                                                                                     | Older. State not checked.                                          |
| [Tags binary search and autotag](../../.opencode/plans/2026-03-25-tags-binary-search-autotag.md) | Tag search                                                                                         | Older. State not checked.                                          |
| [Testing and CI overhaul](../../.opencode/plans/2026-03-25-testing-ci-overhaul.md)               | Test and CI structure                                                                              | Older. State not checked.                                          |
| [HTMX to Datastar migration](../../.opencode/plans/htmx-to-datastar-migration.md)                | Web UI framework move                                                                              | State not checked.                                                 |
| [ProseMirror content modes](../../.opencode/plans/prosemirror-content-modes.md)                  | Editor modes                                                                                       | State not checked.                                                 |

Rows marked "State not checked" were listed from the folder contents. Nobody has compared them with the code yet.
