# Workflow templates

Templates for the files and GitHub issues the [research-to-delivery workflow](../README.md) produces, and for the prompts its orchestrators give subagents. Each template opens with when to use it, then holds the text to copy in one fenced block. Replace every `<...>`; keep the headings, markers and section order.

## Filling in

- IDs come from the tool (`init`, `ids`, `item add`). Never invent `C001`, `C001.2`, `T001` or `D001`.
- Some files are written by a command: concept stubs (`init`), notes (`note`), receipts (`resolve-note`), item lines (`item add`, `item set-status`) and generated views (`render`). Their templates show the shape; use the command.
- Only `render` writes between `<!-- workflow:view <name> -->` and `<!-- /workflow:view <name> -->`. Generated markdown uses headings, `-` bullets, `**bold**`, backticks, links and blank lines; no tables, no `*emphasis*`.
- Package files link relatively (`concepts/C001-<slug>.md`, `/planning/workflow/stages.md`). Issue bodies use full URLs (`https://github.com/<issue_repository>/blob/<trunk>/...`), because an issue is read outside the tree. `<issue_repository>`, `<trunk>`, `<git_env>` and `<worktree_root>` are keys of the [profile](../profile.md).
- Run `prettier --write` on every filled markdown file and `python3 scripts/research_workflow.py check --package <package path>` before committing.

## Package files

Written into `doc/<UTC>_research_<slug>/` ([layout](../records.md)).

| Template                           | File                            | Written by                                 |
| ---------------------------------- | ------------------------------- | ------------------------------------------ |
| [main.md](main.md)                 | `main.md`                       | `init` at S0, then PRs                     |
| [design.md](design.md)             | `design.md`                     | `init` at S0, then PRs; S5 fills stages    |
| [changes.md](changes.md)           | `changes.md`                    | every writer, one line per change          |
| [concept.md](concept.md)           | `concepts/C001-<slug>.md`       | `init`, then only the concept's owner      |
| [note.md](note.md)                 | `notes/C001__to__C002__<id>.md` | `note`, by the sender, once                |
| [receipt.md](receipt.md)           | `receipts/<note-id>.md`         | `resolve-note`, by the receiver            |
| [decision.md](decision.md)         | `decisions/D001-<slug>.md`      | S6 workers and decision subagents          |
| [human-review.md](human-review.md) | `human-review.md`               | S6 orchestrator, from the decision records |

## Issues

Titles follow the table. The marker's `stage` is the stage that creates the issue; the issue is worked at the next one.

| Template                                         | Title                                                                                                              | Created | Worked | Marker `kind`   |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------ | ------- | ------ | --------------- |
| [issue-research.md](issue-research.md)           | `Research: <concept> (<package>·C001)`                                                                             | S1      | S2-S3  | `research`      |
| [issue-parent.md](issue-parent.md)               | `Handoff: Investigate and plan <concept> (<package>·C001)`                                                         | S4      | S5     | `handoff`       |
| [issue-child.md](issue-child.md)                 | `Design: <facet> (<package>·C001.3)`                                                                               | S5      | S6     | `design`        |
| [issue-orchestration.md](issue-orchestration.md) | `Handoff: Orchestrate <design or implementation> of <concept> (<package>·C001) as an orchestrator using subagents` | S5, S7  | S6, S8 | `orchestration` |
| [issue-plan.md](issue-plan.md)                   | `Plan: <concept> (<package>·C001)`                                                                                 | S6      | S7     | `plan`          |
| [issue-implement.md](issue-implement.md)         | `Implement: <step> (<package>·C001·step <n>)`                                                                      | S7      | S8     | `implement`     |

Every issue body ends with one marker line, for example `<!-- workflow:issue {"package":"<package>","concept":"C001","stage":"S1","kind":"research"} -->` (child issues add `"item"`, implement issues add `"step"`). Before creating an issue, look for its marker so a retry never makes a duplicate; this prints the URL of any issue that already carries it:

```sh
gh issue list --repo <issue_repository> --state all --limit 1000 --json url,body |
  jq -r --argjson want '<marker JSON>' '.[] | select([.body | scan("<!-- workflow:issue (\\{.*?\\}) -->")[0] | fromjson] | any(. == $want)) | .url'
```

Use existing labels only. Before publishing, give the draft body, and nothing else, to the `zero-context reader` and fix every gap it finds.

## Prompts

Prompts an orchestrator gives to subagents, in [prompts/](prompts/).

| Prompt                                     | Launched by                | When                                                 |
| ------------------------------------------ | -------------------------- | ---------------------------------------------------- |
| `worker-brief.md`                          | any orchestrator           | dispatching one unit of work                         |
| `orchestrator-brief.md`                    | router, a human            | starting an S6 or S8 orchestrator                    |
| `merger.md`                                | S6, S8 orchestrators       | landing a finished worker PR                         |
| [reviewer.md](prompts/reviewer.md)         | S6, S8 orchestrators       | a disagreement (with critic, investigator) or a PR   |
| [critic.md](prompts/critic.md)             | S6 orchestrator            | a disagreement                                       |
| [investigator.md](prompts/investigator.md) | S6 orchestrator            | a disagreement, or a decision the challenge reopened |
| `challenge-pass.md`                        | S6 orchestrator            | before closing S6                                    |
| `zero-context-reader.md`                   | anyone publishing an issue | before creating any issue or long brief              |
