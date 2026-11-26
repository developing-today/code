# OC-SURVEY raw material deposit

Gathered 2026-11-23/24 (API reads 2026-11-24T00:35–01:05Z) by lane OC-SURVEY for the PM book's OpenCode + OpenChamber section (`~/workspace/pm-book/survey/oc-programme.md`, programme developing-today/code#13900). Per Drew's standing rule (2026-11-23): a lane's gathered work is never thrown away into a summary — this is the full record behind the survey.

Method: GitHub reads via `fleet gh api` only (fully paginated, slim `--jq … | @json` projections); repo file reads on the laptop via `~/tools/laptop-ssh.sh` against `~/git/code`, `~/git/opencode`, `~/git/openchamber-fork`. No GitHub connector, no fresh clones.

## Files

| File                                                                               | Contents                                                                                                   |
| ---------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `repo-opencode.json` / `repo-openchamber.json`                                     | Slim repo objects (parent/source/default branch/counts) for the two forks                                  |
| `pulls-closed-oc.json` (1,438) / `pulls-open-oc.json` (9)                          | All closed/open PRs, developing-today-forks/opencode (number, title, author, labels, base/head, merged_at) |
| `pulls-closed-och.json` (1,290) / `pulls-open-och.json` (2)                        | Same, developing-today-forks/openchamber                                                                   |
| `issues-open-oc.json` (39 entries incl. 9 PRs) / `issues-closed-oc.json`           | Issues, opencode fork; open set includes full bodies, closed set is slim (no bodies)                       |
| `issues-open-och.json` (32 entries incl. 2 PRs) / `issues-closed-och.json` (2,553) | Same, openchamber fork                                                                                     |
| `branches-oc.json` (61) / `branches-och.json` (32)                                 | Branch name lists, both forks                                                                              |
| `labels-oc.json` (168) / `labels-och.json` (150)                                   | Label name lists, both forks                                                                               |
| `code-title-opencode.json` (681)                                                   | code repo search `opencode in:title`, issues+PRs, all states, bodies truncated to 1,500 chars              |
| `code-title-openchamber.json` (137)                                                | code repo search `openchamber in:title`, same shape                                                        |
| `code-title-claimant.json` (148)                                                   | code repo search `claimant in:title`, same shape                                                           |
| `code-open-opencode.json` (480)                                                    | code repo search `opencode is:issue is:open` (body mentions included)                                      |
| `code-open-openchamber.json`                                                       | code repo search `openchamber is:issue is:open`                                                            |
| `laptop-readme-excerpts.txt`                                                       | Fork-notice blocks of both fork READMEs + claimant README tail (laptop reads)                              |
| `pm-stack-opencode-claimant-README.md`                                             | Full README of `pm-stack/opencode-claimant` (laptop, `~/git/code`)                                         |
| `pm-stack-opencode-scheduled-jobs-README-and-jobs.md`                              | Full README + `jobs.json` of `pm-stack/opencode-scheduled-jobs`                                            |
| `pm-stack-opencode-usage-export-README.md`                                         | Full README of `pm-stack/opencode-usage-export`                                                            |
| `fetch3.py` / `fetch4.py` / `fetch5.py`                                            | The fetch scripts that produced the dumps above                                                            |
| `generate.py`                                                                      | The generator that built `survey/oc-programme.md` from these dumps                                         |

Known gap: the broad code search `repo:developing-today/code opencode` totals 4,147 items and the GitHub search API caps retrieval at 1,000; the survey therefore defines its code programme set by title match (see survey §3.3) and deposits the title sets + open-mention sets above instead of a partial broad dump.
