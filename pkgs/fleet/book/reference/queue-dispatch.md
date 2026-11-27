# fleet queue: dispatch and sessions

`dispatch`, `session`, and `surfaces` are _not_ broker verbs: they are the absorbed `fleet-place` reference implementation, deciding **where a build job runs** across the fleet's GitHub Actions copies and runner providers, and dispatching it there as the developing-today GitHub App. The surface registry is data (`fleet-surfaces.json`); no surface is hardcoded. Errors in this family print `repo-queue: <msg>` and exit **2**. This `dispatch` is unrelated to `fleet dispatch` / `dispatch-test` at the top level of the binary (separate domains).

Shared context is in [fleet queue: core verbs](queue-core.md).

Sources: [placement.rs](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/placement.rs), [fleet-surfaces.json](https://github.com/developing-today/code/blob/main/pkgs/fleet/fleet-surfaces.json).

## fleet queue dispatch

```text
usage: repo-queue dispatch --repo <owner/name> --command '<cmd>' [--ref REF] [--runner auto|github|blacksmith] [--platform linux|macos] [--container IMAGE] [--surface ID] [--followup HANDLE|OWNER/REPO#RUN_ID] [--push-nix-cache] [--actions-cache KEY:PATH] [--actions-cache-restore-key PREFIX]... [--allow-paid-blacksmith MINUTES] [--laptop-busy] [--dispatch] [--json] [--registry PATH]
```

| Flag                          | Argument                        | Default                           | Meaning                                                                                                                                         |
| ----------------------------- | ------------------------------- | --------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `--repo`                      | `owner/name`                    | **required**                      | Target repo; must match a surface's targets. Missing → exit 2.                                                                                  |
| `--command`                   | command string                  | **required**                      | The command the surface's `fleet-command.yml` workflow runs. Missing → exit 2.                                                                  |
| `--ref`                       | git ref                         | empty (workflow default)          | Git ref to run at.                                                                                                                              |
| `--runner`                    | `auto\|github\|blacksmith`      | `auto`                            | Provider preference; other values die exit 2.                                                                                                   |
| `--platform`                  | `linux\|macos`                  | `linux`                           | Other values die exit 2. An undocumented alias `--macos` exists in source.                                                                      |
| `--container`                 | image name                      | unset                             | Container image for surfaces with `container_input`; empty value dies exit 2.                                                                   |
| `--surface`                   | surface id                      | unset                             | Pin one surface; an unknown id dies (`--surface <id>: no such surface in the registry`), exit 2.                                                |
| `--followup`                  | `HANDLE` or `OWNER/REPO#RUN_ID` | unset                             | Chain this dispatch after a previous run (portable run reference).                                                                              |
| `--push-nix-cache`            | boolean                         | off                               | Ask the workflow to push the Nix cache (needs the surface's cache secrets).                                                                     |
| `--actions-cache`             | `KEY:PATH`                      | unset                             | One Actions cache binding; must be non-empty `KEY:PATH`, else exit 2. (Single value in source despite the plural name.)                         |
| `--actions-cache-restore-key` | prefix                          | unset                             | Restore-key prefixes. **Repeatable.**                                                                                                           |
| `--allow-paid-blacksmith`     | minutes (number)                | `0`                               | Raise the Blacksmith paid cap for this dispatch only; must parse as a number, else exit 2.                                                      |
| `--laptop-busy`               | boolean                         | off                               | Manually mark the laptop busy (the registry's saturation signal for the codespace tier is deliberately a manual flag, "not scraped telemetry"). |
| `--dispatch`                  | boolean                         | off                               | **Actually dispatch.** Without it the verb is a dry run (the default).                                                                          |
| `--json`                      | boolean                         | off                               | Machine-readable decision/session output.                                                                                                       |
| `--registry`                  | file path                       | resolution order (see `surfaces`) | Placement registry file.                                                                                                                        |
| `--assume-owner-load`         | `owner=N[,owner=N…]`            | unset                             | Testing override, labeled as simulation in the output: pretend owner load.                                                                      |
| `--assume-blacksmith-used`    | minutes                         | unset                             | Testing override, labeled as simulation in the output: pretend Blacksmith minutes already used.                                                 |

**Behaviour.** Loads the registry, computes live capacity per surface (owner concurrency caps, macOS caps, Blacksmith free-minute budget, codespace liveness, local session history/stickiness), picks a surface + provider, and prints the placement decision. **Dry-run is the default**: without `--dispatch` nothing is dispatched. With `--dispatch` it workflow-dispatches `fleet-command.yml` on the chosen surface's repo (as the GitHub App installation token when the registry says `dispatch_as` app, else the signed-in user's token), writes a **session record**, and prints its handle: `fp_` + 128-bit random.

Known discrepancy: the help line says `[--ref REF]`; the placement module's frozen-v1 interface comment says `[--ref SHA]`. Cosmetic only; the flag itself is identical. This chapter documents `--ref REF`.

Exit codes: 0 decision printed (dry run) or dispatched · 2 usage / unknown surface / bad flag value (this family's errors are exit 2).

```text
$ fleet queue dispatch --repo developing-today/code --command 'nix build .#fleet' --json | jq -r .mode
dry-run (no session written; pass --dispatch to submit)
$ fleet queue dispatch --repo developing-today/code --command 'nix build .#fleet' --dispatch --json | jq -r .handle
fp_d0a842c9bd33aa2999f5a12393397ab9
```

## fleet queue session status

```text
usage: repo-queue session <status|result|list> [HANDLE|OWNER/REPO#RUN_ID] [--mine] [--json] [--registry PATH]
```

| Flag                    | Argument                                                  | Default                           | Meaning                                                                                                   |
| ----------------------- | --------------------------------------------------------- | --------------------------------- | --------------------------------------------------------------------------------------------------------- |
| `<handle>` (positional) | session handle `fp_…` or portable ref `OWNER/REPO#RUN_ID` | **required**                      | The session to inspect. A portable reference is resolved directly against GitHub without the local store. |
| `--mine`                | boolean                                                   | off                               | Accepted on the session group (used by `list`; see below).                                                |
| `--json`                | boolean                                                   | off                               | Print the record as JSON instead of the one-line text form.                                               |
| `--registry`            | file path                                                 | resolution order (see `surfaces`) | Placement registry file.                                                                                  |

**Behaviour.** Dispatch commits the session locally, then mirrors it to the queue broker on a best-effort basis. For an `fp_` handle, status selects the newer local or broker record by `updated_at` (the broker wins ties). A broker record can seed an empty local store on a second seat. Status then refreshes the provider state and prints the handle, state, surface and run URL. A failed mirror read leaves the local path available.

Help: a help flag after the subcommand prints the session usage and exits 0 — `session status --help` asks for help; it does not look up a session named `--help` (#13175). The flags documented here come from source.

Exit codes: 0 ok · 2 usage / no such handle / missing handle (`session status needs a HANDLE`).

```text
$ fleet queue session status fp_d0a842c9bd33aa2999f5a12393397ab9
fp_d0a842c9bd33aa2999f5a12393397ab9: state=running surface="fleet-org" run=https://github.com/developing-today/fleet/actions/runs/45146933211
```

## fleet queue session result

```text
usage: repo-queue session <status|result|list> [HANDLE|OWNER/REPO#RUN_ID] [--mine] [--json] [--registry PATH]
```

| Flag                    | Argument                                                  | Default                           | Meaning                                                     |
| ----------------------- | --------------------------------------------------------- | --------------------------------- | ----------------------------------------------------------- |
| `<handle>` (positional) | session handle `fp_…` or portable ref `OWNER/REPO#RUN_ID` | **required**                      | The session to inspect.                                     |
| `--mine`                | boolean                                                   | off                               | Accepted on the session group (used by `list`).             |
| `--json`                | boolean                                                   | off                               | Print the record as JSON instead of the one-line text form. |
| `--registry`            | file path                                                 | resolution order (see `surfaces`) | Placement registry file.                                    |

**Behaviour.** Same refresh as `status`; the output adds `terminal: bool` (`succeeded|failed|cancelled` are terminal) and, when not terminal, the note `not terminal yet — reconnect with the same handle`. Text form: `<handle>: <state> (terminal=<bool>) run=<url>`.

Help: as with `status`, a help flag after the subcommand prints the session usage (see `session status`). Flags are documented from source.

Exit codes: 0 ok · 2 usage / no such handle / missing handle.

```text
$ fleet queue session result fp_d0a842c9bd33aa2999f5a12393397ab9
fp_d0a842c9bd33aa2999f5a12393397ab9: running (terminal=false) run=https://github.com/developing-today/fleet/actions/runs/45146933211
not terminal yet — reconnect with the same handle
```

## fleet queue session list

```text
usage: repo-queue session <status|result|list> [HANDLE|OWNER/REPO#RUN_ID] [--mine] [--json] [--registry PATH]
```

| Flag         | Argument  | Default                           | Meaning                                                           |
| ------------ | --------- | --------------------------------- | ----------------------------------------------------------------- |
| `--mine`     | boolean   | off                               | Filter to records whose `submitted_by` is the current user.       |
| `--json`     | boolean   | off                               | Print `{"sessions": […]}` instead of one status line per session. |
| `--registry` | file path | resolution order (see `surfaces`) | Placement registry file.                                          |

**Behaviour.** Combines the local store with up to 200 broker records, deduplicates by handle and sorts by creation time, newest first. A failed broker read leaves the local records available. Listing shows the recorded state without polling GitHub. Text form is one status line per session, or `no sessions`; `--json` gives `{"sessions": […]}`. Group-level behaviour: `session` with no subcommand prints the usage and returns 2; `session --help` prints usage and returns 0; an unknown subcommand dies exit 2.

Exit codes: 0 ok · 2 usage / unknown subcommand.

```text
$ fleet queue session list
fp_d0a842c9bd33aa2999f5a12393397ab9: state=running surface="fleet-org" run=https://github.com/developing-today/fleet/actions/runs/45146933211
```

## fleet queue surfaces

```text
usage: repo-queue surfaces [--json] [--registry PATH]
```

| Flag         | Argument  | Default                          | Meaning                 |
| ------------ | --------- | -------------------------------- | ----------------------- |
| `--registry` | file path | resolution order (see Behaviour) | Registry file to print. |

**Behaviour.** Prints the resolved placement registry as pretty JSON, exit 0. The optional `--json` flag selects the same output. `surfaces --help` prints a short usage instead of the registry. Registry resolution order: `--registry <path>` or env `FLEET_PLACE_REGISTRY` → a home copy → the source-tree relative path → the registry **embedded in the binary**. Registry shape (version 2): `{comment, version, laptop: {busy, busy_source, note}, owners: {<owner>: {concurrent_jobs_cap, concurrent_macos_cap, plan, private_minutes_monthly}}, surfaces: […]}`.

The shipped surfaces: `fleet-personal` (github-actions, dezren39/fleet, App installation, active), `fleet-org` (github-actions, developing-today/fleet, user-token dispatch, active; also the Blacksmith host), `fleet-forks` (github-actions, developing-today-forks/fleet, **public targets only**, active), `org-run` (standby, retained for manual use; placement prefers fleet-org), `fleet-org-blacksmith` (kind `blacksmith`, Linux-only fallback; budget 3000 free min/mo, paid cap 0 — paid overage disabled unless a dispatch explicitly supplies `--allow-paid-blacksmith`), `codespace-overflow` (kind `codespace`, status `pending-scope`, max 3 live, 180 core-hours/mo budget; uses existing codespaces only, never creates them), `laptop` (kind `laptop`, `handoff-only` — a placement record only; this adapter never runs shell commands on the laptop). Targets accept exact `owner/repo`, `owner/*`, or `*`. `sticky_queue_threshold: 2` and `selection_priority` (10 for the active runners, 20 for org-run) drive the cascade.

Exit codes: 0 ok · 2 usage / unreadable registry.

```text
$ fleet queue surfaces | head -30
{
  "comment": "Fleet placement registry. Targets accept exact owner/repo names, owner/* patterns, or * for all valid owner/repo paths. …",
  "version": 2,
  "laptop": { "busy": false, "busy_source": "manual-flag", "note": "…" },
  "owners": {
    "developing-today": { "concurrent_jobs_cap": 20, "concurrent_macos_cap": 5, "plan": "free-org", "private_minutes_monthly": 2000 },
    …
  },
  "surfaces": [ … ]
}
```
