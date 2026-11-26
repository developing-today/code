# CLI-13 — Shared CLI registration and compatibility inventory

Issue: #9442 (slice CLI-13 of #9208, also the DEP-05 alias). Status:
design record. It is not an authorization to implement. It decides
how the dependency, note and journal commands are registered in the
`fleet` binary, how help and `fleet capabilities` describe them, and
which existing spellings stay as they are.

## 0. Governing decisions consumed

| Source                    | What this record takes from it                                                                                                                                      |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #9442 slice text          | Owned files; first oracle `pmo_cli_dependency_help_and_json_match_capabilities`; negatives; "additive, no queue rename"; help and inventory in the same PR.         |
| CLI-02 record             | One closed-AST parser in `fleet-queue/src/work/cli.rs`; per-operation declarations; strict rules; usage errors exit 2 with stable codes; registration is CLI-13's.  |
| CLI-12 record             | `fleet work link …` spelling; `repo-queue link add/remove/list` stays until CLI-13 decides; its own test target `tests/work_link_contract.rs`.                      |
| DEP-03 record             | `dependency.add/resolve/remove/restore`; `--blocked-by … --hard`; `--expected-version`; `fleet queue block --dep` unchanged.                                        |
| DEP-04 record             | `fleet dependency graph …` and `fleet dependency remove …` flag sets; register through CLI-13; a bare `#42` only when the work's own repo makes it unambiguous.     |
| JRN-02 record             | Broker verbs `/note-add`, `/note-list`, `/note-edit`, `/note-remove`, new `/note-search`; no CLI spellings of its own.                                              |
| AUTH-01 / AUTH-02 records | `fleet.field-authority/v1`; a public mirror is an explicit, scoped, authorized operation, never automatic.                                                          |
| Census                    | `census/commands.jsonl` (215 rows) and `census/command-slice-map.jsonl`; rows 53, 55–59, 92–97, 133, 135, 171, 184, 192–195 name the commands this slice registers. |

## 1. Verified state on main `54b5fbc0`

- `fleet` dispatches by a hand-written argv match, with no argument
  library. `run_root` (`fleet/src/main.rs`) handles `help`,
  `--version`, `version`, `capabilities`, `init` and `session`, then
  resolves a domain from `fleet_registry::DOMAINS` and calls
  `run_domain`. The `queue` domain calls `fleet_queue::client::run`
  in the same process.
- `fleet_registry` is std-only, with no I/O. `Domain` has `name`,
  `summary`, `capabilities`, `primary`, `aliases` and `subcommands`.
  `subcommands` is documented as "descriptive only". There is no
  `work` and no `dependency` domain.
- `fleet capabilities` exists. It prints `build`, `capabilities`,
  `compiled_capabilities` and one entry per domain: `name`,
  `summary`, `aliases`, `capabilities`, `available` and `state`. It
  has no entry per command or per flag.
- The queue domain's `subcommands` list leaves out four verbs that
  `fleet_queue::client::run` dispatches: `note`, `link`, `meta` and
  `resolution-backfill`. Domain help therefore under-reports them.
- `repo-queue note add|list|edit|remove` and `repo-queue link
add|remove|list` exist on the legacy client parser. They call the
  broker's `/note-*` and `/link-*` verbs.
- A standalone `repo-queue` binary still builds as the rollback path
  (`fleet-queue/src/bin/repo-queue.rs`) and calls the same
  `client::run`.
- `fleet-queue/src/work/cli.rs` does not exist. CLI-02's parser is a
  record only.
- The census row `fleet.capabilities` says `absent` at the pin
  `a038f084`. On main the command is implemented.
- Tests: `fleet/src/main.rs` has a unit `mod tests` with help tests.
  `fleet/tests/` holds `ctx_propagation`, `invocation_ledger`,
  `kv_cli`, `proxy_ssh` and `search_serve`. There is no golden help
  or capabilities test anywhere.
- CI: `fleet-rust-tests.yml` runs the whole fleet workspace nightly
  and on `workflow_dispatch`, never on a pull request.
  `fleet-smoke.yml` runs `smoke.sh` on push to main.
- DEP-02 (#9503) has no record on main yet. CLI-02, CLI-12, DEP-03,
  DEP-04, JRN-02, AUTH-01 and AUTH-02 records are on main; none of
  their code is.

## 2. Intent

One registration path puts the dependency and note commands into
`fleet`, so that help, `fleet capabilities` and the parser always say
the same thing. Nothing is advertised that the build cannot do, no
alias appears that help does not list, and every existing `fleet
queue` and `repo-queue` spelling keeps working unchanged.

## 3. Ownership

- **Owned:** `pkgs/fleet/crates/fleet/src/main.rs`,
  `pkgs/fleet/crates/fleet-registry/src/lib.rs`,
  `pkgs/fleet/crates/fleet-queue/src/lib.rs`, the new
  `pkgs/fleet/crates/fleet/tests/cli_registration.rs` and its goldens
  under `pkgs/fleet/crates/fleet/tests/golden/cli/`, and additive
  changes to `census/commands.jsonl`.
- **Needed edits outside the owned list, named:**
  1. One exported `pub const VERBS: &[&str]` in
     `fleet-queue/src/client.rs`, which its own dispatch match is
     tested against, so the registry can be checked against it (D6).
  2. One step in `.github/workflows/fleet-rust-tests.yml` that lists
     the CLI-13 tests by name before running them (§6).
- **Read-only:** `fleet-queue/src/work/cli.rs` (CLI-02), every
  backend module behind the seam, the broker.
- **Added nowhere:** no new parser, argument library, binary, argv0
  name, queue, database, service, resolver or GitHub client.

## 4. Decisions

### D1. Two new domains, one parser

`fleet_registry::DOMAINS` gains two rows, after `queue`:

| Domain       | Summary (help line)                                      | Primary capability | argv0 aliases |
| ------------ | -------------------------------------------------------- | ------------------ | ------------- |
| `work`       | Work items: lifecycle, notes and links.                  | `Capability::Work` | none          |
| `dependency` | Typed dependencies between work items: add, list, graph. | `Capability::Work` | none          |

- `Capability::Work` is a new atom with a new, stable id `work`. It
  is compiled in exactly when the `client` feature of `fleet-queue`
  is, so it follows `Capability::Queue` in the `Full` and `Connect`
  variants and is absent from `Serve`. The existing
  `capability_ids_are_unique_and_stable` test covers the new id.
- `run_domain` gains one arm per domain. Each calls
  `fleet_queue::work::cli::run(domain, args)`, the single CLI-02
  entry point. `dependency` is an operation group inside the same
  closed AST, not a second parser.
- `fleet-queue/src/lib.rs` re-exports `work::cli` under the
  `client` feature, so `fleet` does not reach into a private path.
- Neither domain gets an argv0 alias. There is no `fleet-work`,
  `repo-work` or `fleet-dependency` name.

### D2. The parser's declarations are the single source

CLI-02 §3.2 puts every operation's positionals, flags, value kinds,
exclusions and repeatability in one place in `cli.rs`. CLI-13 does
not copy that table. Instead:

- `cli.rs` exports `pub fn declarations() -> &'static [OperationDecl]`
  (CLI-02 owns the type; CLI-13 only reads it).
- `fleet help work`, `fleet help dependency`, `fleet work --help`,
  `fleet work <op> --help` and the `commands` array of `fleet
capabilities` are all rendered from `declarations()` by code in
  `main.rs`.
- `fleet_registry` stays std-only and keeps only the domain rows. It
  never gains a per-flag table, so there is nothing to drift.
- Adding a later operation touches `cli.rs` and its backend only. It
  needs no edit to `main.rs` or `fleet_registry`, which keeps CLI-13
  the one writer of the shared files.

### D3. `fleet capabilities` is extended, not replaced

The existing JSON keeps every field it has, with the same meaning.
Two fields are added:

- `"schema": "fleet.capabilities/v1"` at the top level.
- `"commands": [ … ]`, one entry per registered operation:

| Field            | Content                                                                                                                                  |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `key`            | The census key, e.g. `dependency.add`.                                                                                                   |
| `domain`         | `work`, `dependency` or `queue`.                                                                                                         |
| `spelling`       | The exact argv, e.g. `fleet dependency add`.                                                                                             |
| `aliases`        | Every other accepted spelling; empty when there is none.                                                                                 |
| `state`          | The same four states as a domain: `available`, `not-in-build`, `narrowed-by-name`, `disabled-by-config`.                                 |
| `flags_declared` | `true` when the entries below are the parser's full declaration; `false` for legacy queue verbs.                                         |
| `flags`          | For each flag: `name`, `aliases`, `value` (`bool`, `string`, `decimal`, `enum`, `path`), `values` for an enum, `required`, `repeatable`. |
| `positionals`    | For each: `name`, `value`, `required`.                                                                                                   |

- Queue verbs appear in `commands` with `flags_declared: false` and
  an empty `flags` list. Their flags live in the legacy client's
  `parse_verb_args` calls, and claiming them here would be a guess.
  They gain `flags_declared: true` only when CLI-02 migrates a verb.
- `--json` is not required for this command; it is already JSON.

### D4. Only what the build can do is advertised

- A command appears in help and in `commands` only when its parse →
  seam path is compiled into this build. A declared operation whose
  backend has not landed is not registered: it is absent, and its
  spelling is `unknown_operation`, exit 2.
- A flag appears only when the operation's declaration accepts it in
  this build. `--mirror` (census row 184) is the case named by the
  slice: it is declared only once AUTH-02's explicit mirror operation
  is wired. Before that it is absent from help and JSON, and the
  parser rejects it as `unknown_argument`.
- So the inventory grows one landing at a time. Each owning slice's
  implementation PR adds its declaration, its backend and its census
  status change together.

### D5. Spellings this record fixes

| Census key                                      | Spelling stem                                | Semantics owner |
| ----------------------------------------------- | -------------------------------------------- | --------------- |
| `dependency.add`                                | `fleet dependency add`                       | DEP-02, DEP-03  |
| `dependency.list`                               | `fleet dependency list`                      | DEP-02          |
| `dependency.remove`                             | `fleet dependency remove`                    | DEP-04          |
| `dependency.graph`                              | `fleet dependency graph`                     | DEP-04          |
| `dependency.blockers`                           | `fleet dependency blockers`                  | DEP-03          |
| `dependency.ready`                              | `fleet dependency ready`                     | DEP-03, SEL-01  |
| `work.block`, `work.unblock`                    | `fleet work block`, `fleet work unblock`     | DEP-03          |
| `work.note.add`, `work.note.list`, `.edit`      | `fleet work note add`, `list`, `edit`        | JRN-02, AUTH-02 |
| new rows `work.note.remove`, `work.note.search` | `fleet work note remove`, `search`           | JRN-02          |
| `work.link`                                     | `fleet work link`, as the CLI-12 record says | CLI-12          |

Full synopses (brackets are optional, a bar separates alternatives):

```text
fleet dependency add <work-ref> (--blocked-by <ref> | --blocks <ref>) [--hard]
    [--expected-version N] [--dry-run] [--json]
fleet dependency list <work-ref> [--blocked-by] [--blocks] [--json]
fleet dependency remove <work-ref> --edge <edge-id> --reason CODE
    --expected-version N [--dry-run] [--json]
fleet dependency graph <work-ref> [--relation R]... [--direction up|down|both]
    [--depth N] [--include-resolved] [--cursor TOKEN] [--json]
fleet dependency blockers <work-ref> [--json]
fleet dependency ready [--limit N] [--json]
fleet work block <work-ref> --blocked-by <ref> [--hard]
fleet work unblock <work-ref>
fleet work note add|list|edit <work-ref> ...
fleet work note remove <work-ref> --note <note-id>
fleet work note search <query> [--work <work-ref>]
```

Rules behind the table:

1. **Work refs are DEP-02's target refs.** The `<work-ref>` and
   `<ref>` arguments take exactly the forms DEP-02's record defines,
   matching `WorkRef::qualified_uri` in `work/contracts.rs`: the
   qualified `fleet://<authority_id>/work/<work_id>`, and the
   shorthands `fleet://work/<work_id>` and `fleet:<work_id>` (as in
   DEP-03's `fleet:761`), which are accepted only when the
   authenticated authority context is unambiguous. CLI-12's record
   writes `<authority>/<work_id>`; none of CLI-12's code exists, so
   its implementation takes these forms instead, and the bare `/`
   pair is not accepted. `o/r#n` stays the GitHub issue and PR form,
   so `/` never means two things.
2. **No bare numbers.** CLI-02 rejects a bare numeric target. The
   charter's example `fleet dependency add 842 --blocked-by fleet:761
--hard` is therefore registered as `fleet dependency add fleet:842
--blocked-by fleet:761 --hard`; `842` alone is
   `ambiguous_target_form`, exit 2. A bare `#42` for a GitHub
   reference follows DEP-04: accepted only when the work's own repo
   makes it unambiguous, decided behind the seam.
3. **Two-word operations.** `note` is an operation group: `fleet work
note` needs one more token from the closed set `add`, `list`,
   `edit`, `remove`, `search`. A missing or unknown verb is
   `unknown_operation`. This narrows CLI-02 §3.1's single-token
   operation to "a token, or a group token plus one verb token". It
   adds no prefix matching.
4. **`--blocked-by` and `--blocks`** are mutually exclusive on `add`,
   and one is required. Their meaning is DEP-02's.
5. **New census rows.** `work.note.remove` and `work.note.search` are
   appended as new rows, owner 9208, slice JRN-02, status `absent`,
   when their declarations land.

### D6. Existing spellings stay, and are listed honestly

- Every `fleet queue …` verb, every `repo-queue …` spelling, the
  standalone `repo-queue` binary and `fleet queue block --dep` stay
  exactly as they are. Nothing is renamed or removed.
- `fleet queue note …` and `fleet queue link …` keep calling the
  same broker verbs as `fleet work note …` and `fleet work link …`
  will. They are separate spellings on separate parsers, not
  aliases of each other, and help says so in one line on each.
- The queue domain's `subcommands` list gains `note`, `link`, `meta`
  and `resolution-backfill`. The unit test
  `queue_registry_subcommands_match_client_dispatch` asserts the list
  equals `fleet_queue::client::VERBS` as sets, so neither side can
  drift again.
- `expedite=15` and `next-up=25` stay distinct from `do-now`.

### D7. Census changes

- Rows are never deleted, and `key`, `spelling` and `owner` are never
  rewritten.
- A row's `status` and `evidence` change only in the PR that lands
  the registration they describe, and that change adds an
  `observed_at` field with the main commit it was checked at.
- The CLI-13 implementation PR corrects `fleet.capabilities` this
  way: `status: implemented`, evidence `fleet/src/main.rs
print_capabilities`, `observed_at` the main commit. It adds the
  same for `fleet.help`'s evidence line, which points at a stale line
  number.

### D8. Errors and exit codes

- Parse errors use CLI-02's codes (`unknown_argument`,
  `unknown_operation`, `missing_value`, `duplicate_flag`,
  `conflicting_flags`, `malformed_value`, `ambiguous_target_form`,
  `unrecognized_trailing_input`) and exit 2, before any broker call.
- With `--json` anywhere in argv, the parse error is printed as one
  JSON object `{code, argument, usage}` on stderr. Without it, the
  same facts print as text.
- `fleet <unknown-domain>` keeps its current `usage_error` text and
  exit status.

## 5. Negative cases

| Negative (slice text)         | How it is prevented                                                                                                 | Test                                                                            |
| ----------------------------- | ------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| Silent aliases                | No new argv0 name; every accepted alternative spelling is in `aliases` and in help; no prefix matching.             | `cli_registration_has_no_silent_aliases`                                        |
| Unsupported mirror advertised | `--mirror` is declared only when AUTH-02's operation is compiled; otherwise absent and rejected (D4).               | `cli_registration_does_not_advertise_unwired_mirror`                            |
| Missing options               | Help and JSON render the same declaration; every declared flag appears in both.                                     | `pmo_cli_dependency_help_and_json_match_capabilities`                           |
| Unknown flags accepted        | `fleet dependency add fleet:1 --blocked-by fleet:2 --bogus` exits 2 with `unknown_argument` and contacts no broker. | `cli_registration_rejects_unknown_flags_before_any_call`                        |
| Queue spelling drift          | Goldens of `fleet queue --help` and `fleet help queue`; registry list equals the client's verbs.                    | `queue_spellings_unchanged`, `queue_registry_subcommands_match_client_dispatch` |
| Bare number accepted          | `fleet dependency add 842 …` exits 2 with `ambiguous_target_form`.                                                  | `cli_registration_rejects_bare_work_number`                                     |

## 6. Oracles and tests

New integration target `pkgs/fleet/crates/fleet/tests/cli_registration.rs`
in the `fleet` crate. It runs the built binary through
`env!("CARGO_BIN_EXE_fleet")` with the queue broker address pointed
at a closed local port, so any broker call fails loudly with a
different error from a parse error.

| Test                                                     | Asserts                                                                                                                                                                      |
| -------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pmo_cli_dependency_help_and_json_match_capabilities`    | For every `commands` entry with domain `dependency`, the flag and positional set in `fleet dependency <op> --help` equals the JSON entry, both ways; both match the goldens. |
| `cli_registration_has_no_silent_aliases`                 | Every name `is_known_invocation_name` accepts appears in root help; every alias in JSON appears in that command's help.                                                      |
| `cli_registration_does_not_advertise_unwired_mirror`     | In a build without AUTH-02's mirror operation, `--mirror` is in no help text and no JSON entry, and passing it exits 2.                                                      |
| `cli_registration_rejects_unknown_flags_before_any_call` | Exit 2, code `unknown_argument`, and the closed-port error never appears.                                                                                                    |
| `cli_registration_rejects_bare_work_number`              | Exit 2, code `ambiguous_target_form`.                                                                                                                                        |
| `queue_spellings_unchanged`                              | `fleet queue --help`, `fleet help queue` and the `repo-queue` route match their goldens.                                                                                     |
| `capabilities_json_keeps_existing_fields`                | Every field `fleet capabilities` prints at `54b5fbc0` is still present with the same type.                                                                                   |

Unit test in `fleet/src/main.rs` `mod tests`:
`queue_registry_subcommands_match_client_dispatch`.

Goldens live in `tests/golden/cli/`: `help-dependency.txt`,
`help-work.txt`, `help-queue.txt`, `capabilities.json`. The build
fields that change per commit (`version`, `derived_version`,
`source_rev`) are replaced with fixed placeholders before comparison.
Regenerating a golden is a reviewed diff, never an automatic update.

Done check, as the slice states:

1. `cargo test -p fleet --test cli_registration -- --list` shows each
   test above by exact name; a missing name fails the step. A filter
   that matches zero tests is not a pass.
2. Then that target runs.
3. `cargo test -p fleet-queue --lib` runs the fleet-queue unit
   contract tests separately.

All three run remotely by dispatching `fleet-rust-tests.yml` on the
branch, with the listing step added (§3, named edit 2). Nothing is
compiled on a laptop or sandbox.

The first red test is `pmo_cli_dependency_help_and_json_match_capabilities`:
on main it fails because there is no `dependency` domain and no
`commands` array.

## 7. Compatibility and reversal

Additive. Existing domains, verbs, flags, argv0 names, exit codes and
JSON fields keep their meaning. Reversal is to remove the two domain
rows and the two `run_domain` arms: `fleet work` and `fleet
dependency` then return `unrecognized subcommand`, the `commands`
array disappears, and every queue path is untouched. No stored state,
broker verb, queue row or history is touched by registration.

## 8. Requirement trace

| Requirement                    | How this record serves it                                                                                                                             |
| ------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| C-R005, C-R107, O-R023, P-R003 | One cohesive `fleet` surface: two new domains on the existing registry and one parser (D1, D2).                                                       |
| C-R108, O-R024                 | Shared flags appear only where an operation's declaration lists them, and help and JSON show the same set (D2, D3).                                   |
| C-R109, C-R110                 | Domains are spellings into one seam; no second backend per spelling (D1).                                                                             |
| C-R009, C-R106                 | Dependency and note commands become discoverable through help and `fleet capabilities` (D3, D5).                                                      |
| C-R087                         | Slash spellings stay CLI-02's; this record registers argv only and adds no slash parser.                                                              |
| C-R149–C-R153, C-R156, H-R016  | Scenario B's `dependency add … --blocked-by … --hard` and `work unblock` spellings are fixed (D5); scenarios run against the composed seam, not here. |
| H-R017, H-R024, H-R031         | Every existing command and option is kept and goldened; new ones are added rows, never renames (D6, D7, §6).                                          |
| P-R005                         | Work refs carry an authority or a shorthand bound to one; never a bare number (D5 rules 1–2).                                                         |
| P-R009, P-R012, P-R024, P-R028 | The dependency, note and link families are registered under their census keys, with semantics left to DEP, JRN and CLI-12 (D5).                       |

## 9. Gates

1. This record merged.
2. CLI-02's parser (`work/cli.rs` with `declarations()` and `run`)
   on main. Until then CLI-13 has nothing to route to.
3. DEP-02's record (#9503) merged, for `--blocked-by` and `--blocks`
   semantics. JRN-02 and AUTH-01 records are already on main.
4. The CLI-13 implementation PR can then land D1–D3, D6, D7, D8 and
   the tests with only the operations whose backends exist. Under D4
   it may land with the `dependency` domain present and its operation
   list empty; the oracle then compares two empty sets for that
   domain plus the goldens, and gains entries as each backend lands.
5. Live acceptance, deployment and any production write stay with
   their owning slices.

## 10. Non-goals

- The parser, its AST or its error codes (CLI-02).
- Dependency, note, link or mirror semantics (DEP-02/03/04, JRN-02,
  CLI-12, AUTH-02).
- Slash-command grammar or routing (CLI-02 §5, DSP slices).
- Migrating legacy `fleet queue` verbs onto the CLI-02 parser.
- Removing the standalone `repo-queue` binary or any queue spelling.
- An exit-code ↔ outcome matrix beyond parse failures (CLI-02 §7).
