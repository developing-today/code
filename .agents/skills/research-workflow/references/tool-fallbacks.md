# When a workflow command is missing

Read this only when `python3 -I scripts/research_workflow.py <command>` fails with `invalid choice` or an unknown flag. That means the tool on your base branch is older than the command set these skills rely on (the workflow docs in [planning/workflow/](../../../../planning/workflow/README.md) describe it). In that case:

- record `tool behind skill: <command>` in your turn report under Questions, with the default "used the fallback below";
- use the fallback below;
- don't edit the tool unless you own it.

- **`status --live`**: run `rw status --package "$PKG"` for stage and status. Derive the run status yourself:
  - `git ls-remote origin 'refs/heads/research/<slug>/*'` shows pushed branches;
  - `gh pr list --state all --search "<package id> in:body" --json number,headRefName,headRefOid,state,url` shows PRs;
  - `git merge-base --is-ancestor <sha> origin/main` shows what has merged.
- **`ready` without waves**: use the plain `ready` output, then drop concepts that hold a lease or have a running worker.
- **`lease acquire|release|list`**: use the `mkdir` lock in collaboration rule 5 ([collaboration.md](../../../../planning/workflow/collaboration.md)): `mkdir "$D/locks/<resource>.lock"`, then write `owner` as `<name> <acquired> <expires> <purpose>`. An existing directory means someone else holds the lock.
- **`item add|set-status`**: edit the items list in `main.md` by hand, following the trail grammar in [records.md](../../../../planning/workflow/records.md), and allocate the next free child number under the parent. Then add one `changes.md` line.
- **`ids`**: `grep -o 'C[0-9]\{3\}\(\.[0-9]\+\)*' "$PKG/main.md" | sort -uV`.
- **`render`**: leave the generated views as they are and note it in the turn report. Never hand-edit text between `workflow:view` markers.
- **`verify-transcript --against`**: `cmp "$SRC" "$PKG/transcript.txt" && sha256sum "$SRC" "$PKG/transcript.txt"`.
- **`init --preview-segments`**: the older tool works by line. Run `nl -ba "$SRC"` and write `source_lines: [[start, end]]` ranges in the outline instead of `segments`. The test fixture `outline()` in `scripts/tests/test_research_workflow.py` is the example.
- **`check --remote-notes`**: `git fetch -q origin`, then `git log --name-only --format= origin/main..origin/<branch> -- "$PKG/notes/"` for each research branch.
- **`check --changed-since <ref>`**: `rw check --package "$PKG"`, which checks the whole package.
