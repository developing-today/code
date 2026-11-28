# C010: Thin Orchestration, Merge Verification, and Worker Lifecycles

[Main reference](../main.md#c010)

## Relevant transcript excerpts

<!-- workflow:excerpt C010 53 55 -->

[Transcript lines 53-55](../transcript.txt#L53-L55)

```text
The spawning agent for these sub-agents that is managing the issues from this file should avoid implementing. Its main purpose is to:
- ensure each sub-agent completes, reviews and validates their changes, merges their PRs, and restarts them if they stopped without completing or pushing
- make preliminary decisions based on the research each subagent does
```

## Normalized purpose

Enforce thin orchestrators that manage ready frontiers, verify worker commit SHAs before merging, and restart stalled workers without implementing.

## Initial thoughts

- Orchestrators must avoid implementation drift; their duties are coordination and merge gating.
- Merges are executed only when remote SHA matches PR head matches worker reported SHA.

## Inferred suggestions

- Inference: Periodically sweep for orphan branches created when workers push after PR auto-delete.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
