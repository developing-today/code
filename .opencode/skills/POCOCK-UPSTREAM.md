# Vendored Matt Pocock skills

The skills listed below are copied from Matt Pocock's skills repository. The other skills in this directory (`autoresearch`, `jules-*`, `mcpx-*`) are not his and are not covered here.

| Field    | Value                                                                                                                        |
| -------- | ---------------------------------------------------------------------------------------------------------------------------- |
| Source   | https://github.com/mattpocock/skills                                                                                         |
| Commit   | `5beb35b040e3de97e54fa9e386d7e42d15c295c2` (2026-10-19, "chief-of-staff: keep memory in a dated, commit-cited wiki (#1304)") |
| Vendored | 2026-10-21                                                                                                                   |
| License  | MIT, Copyright (c) 2026 Matt Pocock (full text below)                                                                        |

Layout: upstream `skills/<category>/<name>/` becomes `.opencode/skills/<name>/`, flattened, with every file kept (`SKILL.md`, supporting docs and scripts, `agents/openai.yaml`).

## Vendored skills

| Upstream category | Skills                                                                                                                                                                                                                                                             |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| engineering       | ask-matt, code-review, codebase-design, diagnosing-bugs, domain-modeling, grill-with-docs, implement, implement-spec, improve-codebase-architecture, pr, prototype, research, retro, setup-matt-pocock-skills, tdd, to-spec, to-tickets, triage, wayfinder, wizard |
| productivity      | grill-me, grilling, handoff, teach, to-questionnaire, wait-what, writing-for-agents                                                                                                                                                                                |
| in-progress       | claude-handoff, loop-me                                                                                                                                                                                                                                            |
| misc              | git-guardrails-claude-code                                                                                                                                                                                                                                         |

Not vendored: in-progress `chief-of-staff`, `setup-ts-deep-modules`, `writing-beats`, `writing-fragments`, `writing-shape`; misc `migrate-to-shoehorn`, `scaffold-exercises`, `setup-pre-commit`. No vendored skill calls them.

## Local changes

Every vendored file is byte-identical to the commit above except:

- `claude-handoff/SKILL.md`: the first paragraph says how to write the summary file (file-writing tool, or a quoted heredoc from the shell, never `echo`) and to keep the double quotes around `"$(cat <summary file>)"`. Upstream already passes the summary through a temp file, but leaves how the file gets written open.

## Re-syncing

1. Clone the upstream repo into an empty directory and note its HEAD.
2. For each skill here, `diff -r .opencode/skills/<name> <clone>/skills/*/<name>`.
3. Before overwriting a differing file, check whether its blob (`git hash-object`) appears in upstream history. If it doesn't, the file has a local edit, so carry it forward and list it under Local changes.
4. Update the commit, date and lists in this file.

## License

```
MIT License

Copyright (c) 2026 Matt Pocock

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
