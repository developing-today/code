# C009: Worktree Boot Protocol and Merge-Commit Pull Requests

[Main reference](../main.md#c009)

## Relevant transcript excerpts

<!-- workflow:excerpt C009 49 51 -->

[Transcript lines 49-51](../transcript.txt#L49-L51)

```text
One should push a branch and create a new work tree for each subagent and execution that occurs at this stage. They should only be sharing when the subagents are working on distinct files, or the subagents aren't writing, or there's a minimal chance of conflict that could be easily discovered.

The first thing that should happen in each of these subagents is that they boot in a work tree in the appropriate branch of its main or some feature. They should immediately create a new branch and push the branch with no changes so that subagents can create and merge PRs. These PRs don't need to have heavy review. The main purpose is to prevent conflicts and ensure that they're handled before merge. They don't need to fix preexisting issues and should only use merge commits.
```

## Normalized purpose

Execute parallel workers in isolated git worktrees, immediately pushing empty branches, creating draft PRs, and landing via merge commits.

## Initial thoughts

- Pushing the branch immediately reserves the ref on origin.
- Merge commits (--no-ff) preserve historical development context and eliminate rebase churn.

## Inferred suggestions

- Inference: Source the local git-env.sh and disable worktree auto-gc during the boot sequence.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
