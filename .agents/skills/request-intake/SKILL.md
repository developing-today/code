---
name: request-intake
description: Capture a request, from one sentence to a multipage rambling or speech-to-text transcript, as a research package (stage S0). Stores the exact bytes with the workflow tool's init, maps every sentence segment to a concept or the global bucket G, keeps hints, contradictions and changes of mind, records garbled words in a reading register, explores the repo for context and overlap with parallel read-only subagents, pre-creates one stub per concept and lands main.md, design.md and changes.md as a docs-only PR. Use when the user hands over new ideas, a brain dump, a voice note or a long request to turn into research and design docs, or says capture, intake or start a research package. Stops after intake unless told to continue. For a package that already exists, load research-workflow instead.
---

# Request intake (S0)

This skill turns one request into a research package. The request can be anything from a sentence to a multipage voice transcript full of typos. The package holds:

- the exact transcript;
- a manifest;
- the main reference;
- the meta design;
- a change log;
- one stub per concept.

One agent runs intake. Intake keeps everything; refining, merging and rejecting come at S2 and later. The layout and formats are in [records.md](../../../planning/workflow/records.md), the stages in [stages.md](../../../planning/workflow/stages.md), and the vocabulary (types, relations, priority, horizon) in [DEPENDENCIES.md](../../../planning/DEPENDENCIES.md).

## Set up

```sh
source /home/user/.local/share/plaza-tools/git-env.sh                 # the profile's git_env
SLUG=<kebab-slug>; TS=$(date -u +%Y-%m-%dT%H:%M:%SZ); PKG="doc/${TS//:/-}_research_$SLUG"
git fetch -q origin
git worktree add -b "research/$SLUG/s0-intake" "/home/user/work/wt/research-$SLUG-s0-intake" origin/main
cd "/home/user/work/wt/research-$SLUG-s0-intake" && git push -u origin "research/$SLUG/s0-intake"
rw() { python3 -I scripts/research_workflow.py "$@"; }
WORK="$(git rev-parse --git-common-dir)/agent-work/${PKG##*/}-S0"; mkdir -p "$WORK"   # scratch, never committed
```

If a command fails with `invalid choice` or an unknown flag, read [tool-fallbacks.md](../research-workflow/references/tool-fallbacks.md).

## Steps

1. **Get the exact bytes without retyping.** Use the source as it was delivered, chosen with [references/exact-bytes.md](references/exact-bytes.md): a file path, an attachment, an issue body, a pipe or the harness's own record of the message. Set `SRC` to that file and `SHA=$(sha256sum "$SRC" | cut -d' ' -f1)`. If the user or the source supplied a hash, it is the expected hash. Scan the input for secrets (keys, tokens, passwords). This repository is public.
   Done when `SRC` holds the delivered bytes, the source is described in one line, any expected hash is noted, and the secret scan is clean. A hit makes you Blocked on user.

2. **Preview the segments.** Run `rw init --input "$SRC" --slug "$SLUG" --preview-segments > "$WORK/segments.txt"`. Segments are sentences: the tool splits after `.`, `?` and `!`, and at every newline.
   Done when `segments.txt` lists `T001` to `Tn` and `git status --short` shows the preview wrote nothing to the repository.

3. **Read everything and record what it says.** Read every segment, following [references/reading-messy-input.md](references/reading-messy-input.md). For long input it splits the work across parallel read-only range readers. Write `$WORK/reading.md` with these lists, each entry citing segment ids:
   - candidate concepts;
   - directives about how to work (these go to `G`);
   - hints;
   - contradictions;
   - changes of mind;
   - repeats;
   - garbled tokens;
   - ideas the user missed or only implied.
     Done when every segment id appears in `reading.md` at least once.

4. **Explore the context in parallel (read-only).** In one turn, launch the explore subagents in [references/explore-briefs.md](references/explore-briefs.md):
   - code and config;
   - docs and architecture;
   - issues and PRs;
   - one more for each large area the topics touch.

   Each returns `file:line` evidence per topic, plus an overlap list of existing docs, issues or PRs that already cover a topic, contradict it, or would be edited by it. Before using a report, re-run two of its claims.
   Done when every candidate concept has evidence or a "nothing found (searched: ...)" line, and the overlap list is merged into `reading.md`.

5. **Write the outline.** Write `$WORK/outline.json` using the schema and example in [references/outline.md](references/outline.md):
   - Make one concept per document to create (`kind: new`) or existing file to edit (`kind: edit`, with `targets`).
   - Give every segment to a concept or to `G`. Overlap between concepts is allowed.
   - A hinted idea gets `type: hint`.
   - Keep both sides of a contradiction or change of mind, with `contradicts` or `supersedes`.
   - Put garbled tokens in the reading register.
   - Add missed or half-formed ideas to `suggestions[]`, each starting `Inference:`.
   - Take priority, horizon, goal and `enables` from the user's words, else `unspecified`.
     Done when every idea in a multi-idea segment is claimed by a concept, by `G` or by a suggestion (the idea-level check in `references/outline.md`).

6. **Create the package.**

   ```sh
   rw init --input "$SRC" --slug "$SLUG" --timestamp "$TS" --outline "$WORK/outline.json" \
     --expect-sha256 "$SHA" --source "<one line: where the bytes came from>" --visibility public
   rw verify-transcript --package "$PKG" --against "$SRC"
   ```

   - Add `--encoding <name>` only for input that isn't UTF-8.
   - `init` refuses bytes that are already captured. Hand that package to `research-workflow` instead. Use `--allow-duplicate` only if the user asks for a second capture.
   - Uncovered segments make `init` fail and write nothing. Fix the outline and rerun.

   Done when `init` exits 0 and `verify-transcript` reports a match.

7. **Complete the main reference and the meta design.** Go through [references/package-checklist.md](references/package-checklist.md). In `main.md`, make sure it has:
   - the file list (every stub and every target);
   - the introduction;
   - `G`;
   - the reading register;
   - each concept's preliminary thoughts as hierarchical items;
   - empty Issues and PRs placeholders.

   Add any missing item with `rw item add --package "$PKG" --parent C001 --text "..."`. In `design.md`, make sure it has:
   - the first-pass review order with reasons;
   - the waves from `rw ready --package "$PKG"`;
   - the line "run one agent per concept" where nothing blocks;
   - the run table from `rw render --package "$PKG"`.

   Done when every checklist line holds and `rw ids --package "$PKG"` lists items for every concept that has thoughts.

8. **Validate.** Commit first (`git add -- "$PKG" && git commit -m "docs: capture $SLUG request"`), then run:

   ```sh
   rw check --package "$PKG" && rw check --repo-root .
   prettier --check "$PKG"/*.md "$PKG"/concepts/*.md && biome format "$PKG"/package.json
   rw render --package "$PKG" && git diff --exit-code -- "$PKG"      # views are current and stable
   ```

   Then give `main.md` to a fresh zero-context reader subagent with the prompt in `references/package-checklist.md`. Fix every gap it finds in the package, or list it under Questions.
   Done when every command exits 0 and each of the reader's questions is answered in the package or listed.

9. **Land as a docs-only PR.**
   1. `git fetch -q origin && git merge --no-edit origin/main`, then push.
   2. `gh pr create --base main --title "docs: capture <slug> request" --body-file "$WORK/pr.md"`, with the body from the checklist.
   3. `git diff --name-only origin/main...HEAD` must list only `$PKG/**`.
   4. If you are a dispatched worker, stop here; the router merges.
   5. If you are running standalone, wait for green checks, then `gh pr merge <n> --merge --match-head-commit "$(git rev-parse HEAD)"`, then `git merge-base --is-ancestor <sha> origin/main`.

   Done when the PR is open (worker), or merged with the ancestor check passing (standalone).

10. **Stop at intake.** Continue only if the user asked for more, in which case load `research-workflow` with the package path.
    Done when the turn report names the package, the PR and the next stage (S1, one `research-prepare` agent per ready concept).

## Parallelism and ownership

Intake has one writer: you own the whole package until its PR merges. The range readers and explore subagents are read-only and return reports; they write nothing. Once the PR merges, each concept file belongs to the worker of its current stage. From then on, `main.md` and `design.md` change only through PRs, and `changes.md` only by appending a single line. The rules are in [collaboration.md](../../../planning/workflow/collaboration.md); this skill does not restate them.

## Should load / should not load

- Load for: a new request, brain dump, voice note or transcript to capture; "start a research package"; "store this exactly and break it into docs"; making the main reference, the meta design and the concept stubs for new input.
- Don't load for: an existing package (`research-workflow`); preparing one concept (`research-prepare`); deep research (`concept-research`); transcribing audio; polishing dictated prose; questioning the user about a plan (`grilling`); turning a spec into tickets (`feature-plan`).

## References

- [references/exact-bytes.md](references/exact-bytes.md): read in step 1.
- [references/reading-messy-input.md](references/reading-messy-input.md): read in step 3, and again whenever the input is long, garbled or self-contradicting.
- [references/explore-briefs.md](references/explore-briefs.md): read in step 4. `research-prepare` uses it too.
- [references/outline.md](references/outline.md): read in step 5.
- [references/package-checklist.md](references/package-checklist.md): read in steps 7 to 9.

## Turn report

```
Done: <package path, n segments, n concepts, G size, PR URL>
Verified: <rw check ... -> ok; verify-transcript -> match sha256 <first 12>; prettier --check -> ok>
Remaining: <S1 for C001..Cn, or nothing if the user stopped at intake>
Questions (non-blocking): <readings of garbled input, contradictions kept unresolved -> default used>, or none
Blocked on user: <secret found, expected hash mismatch>, or none
```
