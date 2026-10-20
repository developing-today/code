# Roadmap: repo-wide CI and automation

This file covers `.github/workflows`, the Jules automation and the way pull requests are triaged. Each project's own checks are in its roadmap. How to read this file: [planning README](../README.md).

## Now

| Title                                  | Status   | Intent                                                                          | Links      |
| -------------------------------------- | -------- | ------------------------------------------------------------------------------- | ---------- |
| One Jules workflow                     | proposed | Three Jules workflow files become one.                                          | #287       |
| Cache the Go build and reuse artifacts | proposed | Build cache by `go.sum` hash.                                                   | #288       |
| `nix flake check` out of disk          | proposed | It dies intermittently with "No space left on device" in `id-test` and `id-ci`. | #257       |
| Triage stale PRs                       | proposed | Overlapping Jules drafts, old Renovate security PRs and one PR with conflicts.  | #310, #311 |

## Next

| Title                                         | Status      | Intent                                                                               | Links                           |
| --------------------------------------------- | ----------- | ------------------------------------------------------------------------------------ | ------------------------------- |
| The `id` workflow gate stays under 15 minutes | planned     | The gate once took about two hours. It is fast now. Keep it that way and measure it. | #292, F11.4                     |
| Adopt the planning folder                     | in-progress | New plans and roadmap changes go through `planning/`.                                | [planning README](../README.md) |

## Ideas

| Title                         | Status | Intent                                                              | Links |
| ----------------------------- | ------ | ------------------------------------------------------------------- | ----- |
| Jules does not repeat a theme | idea   | The Bolt and Palette jobs skip a theme that already has an open PR. | #310  |

## Done

| Title                                               | Date       | Result              |
| --------------------------------------------------- | ---------- | ------------------- |
| Concurrency groups on CI workflows                  | 2026-10-18 | #289                |
| Auto-assign concurrency keyed by issue or PR number | 2026-10-18 | #303                |
| Jules runs only when they can act                   | 2026-10-18 | #304                |
| Timeout caps on every job                           | 2026-10-19 | #293                |
| PR gate cannot drift from the flake                 | 2026-10-18 | #318 (closes #300)  |
| `mcpx` and `id` workflow speed-up                   | 2026-10-18 | #295                |
| Package build check when dependencies change        | 2026-10-18 | `mcpx-nixbuild.yml` |
