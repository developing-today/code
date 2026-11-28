# Publishing issues: markers, links, validation

Every mode publishes through these rules. They adapt cairn's handoff goal (`~/git/cairn/prompts/goals/handoff/SPEC.md`, sections 7 to 9) to this workflow.

## Titles and markers

| Mode          | Title                                        | Marker `kind`   | `stage` |
| ------------- | -------------------------------------------- | --------------- | ------- |
| parent        | `Handoff: <concept title> (<package>·C001)`  | `parent`        | `S4`    |
| fanout child  | `Research: <facet title> (<package>·C001.N)` | `child`         | `S5`    |
| orchestration | `Design: <concept title> (<package>·C001)`   | `orchestration` | `S5`    |

The marker is the last line of the body: `<!-- workflow:issue {...} -->`, holding JSON with sorted keys `concept`, `facet` (children only), `kind`, `package` and `stage`. It is unique per issue. If the templates in `planning/workflow/templates/` define more keys, add them; never drop these.

## Duplicate check, in this order

1. Your own record: the concept state block's `issues` and your live log. A URL recorded there is the issue; reuse it.
2. Search, then match exactly. GitHub search splits the marker into words and matches loosely, so a search only narrows the candidates:

   ```sh
   gh issue list --repo <repo> --state all --search '"<package>" "C001" in:body' \
     --json number,url,title,state,body --limit 200
   ```

   Keep only the issues whose body contains the exact marker string.

3. A title search for the exact title, as a last check.

A new issue can take a while to appear in search, which is why step 1 comes first and why you record each URL as soon as `gh issue create` returns it.

When a match is found:

- open, and nobody has started on it (no linked PR, no worker comments): update its body with `gh issue edit <url> --body-file <file>` if yours is better, otherwise reuse it as is;
- open, and work has started: leave the body alone and add a comment with the differences;
- closed: do not reopen it. Record it and stop as blocked if the closure contradicts the package state.

## Labels and people

Run `gh label list --repo <repo> --json name` and add only labels that already exist and fit, for example `documentation` for research and design issues. Never create labels. Never assign people unless the repository's convention says to.

## Links

- Use full GitHub URLs, never relative links: a relative link resolves against the issue URL and breaks.
- Files on `main`: `https://github.com/<repo>/blob/main/<path>`, plus `#<heading-anchor>` when the anchor exists.
- Line references: a permalink at the recorded SHA, `https://github.com/<repo>/blob/<sha>/<path>#L10-L20`, with a note that lines drift.
- Check every file link before publishing: `gh api "repos/<repo>/contents/<path>?ref=<ref>" --silent` exits 0. Content that exists only on an unmerged branch is not linkable yet; wait for its merge or name the PR instead.
- Mention a local path only with what it contains and that it is on one machine.

## Length and safety

- The body must stay under GitHub's 65,536-character limit; check with `wc -c`. If it is longer, move the bulk into a repository file (through your PR) or a first comment, and keep the goal, current state, role, start and done sections in the body.
- No secret values anywhere. This repository is public: nothing private either.

## Zero-context validation

Give the draft body, and nothing else, to a fresh read-only subagent with this prompt (from cairn's `agents/zero-context-reader.md`):

```text
You know nothing except the document below. Use no other context. You may follow its own links to check that they resolve and say what they claim. Do not write files or launch subagents.
Return:
1. Restatement: the goal, your role, and the first three actions you would take.
2. Questions: every question you would need answered before starting, most blocking first.
3. Unlocatable claims: claims you could not find support for in the document or its links.
4. Broken or local-only references.
5. Risks: places where two readings would lead to different work.
<draft body>
```

Fix every gap and re-run until there is no blocking question, no broken link and no unlocatable claim. In fan-out mode, run one validator per child body, in parallel.

## Publish and verify

```sh
gh issue create --repo <repo> --title "<title>" --body-file <file> [--label <existing>] [--parent <number>]
gh issue view <url> --json url,state,title,body
```

Confirm the issue is open, the body is complete (no truncation, every section present) and its last line is the exact marker. Record the URL in your live log at once (`<name> <UTC> issue: <url> <kind> <id>`), then in the concept state block.

If an older open issue covers the same work, comment on each with a link to the other and leave closing to the user. Never close other issues.

## Resuming after a failure

Re-run the duplicate check for every planned issue, reuse what exists, and create only the missing ones. Never create a second issue with the same marker.
