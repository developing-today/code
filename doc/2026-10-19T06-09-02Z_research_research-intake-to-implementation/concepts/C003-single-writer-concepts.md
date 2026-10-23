# C003: Single-Writer Concept Files and Mutation Invariance

[Main reference](../main.md#c003)

## Relevant transcript excerpts

<!-- workflow:excerpt C003 19 19 -->

[Transcript lines 19-19](../transcript.txt#L19-L19)

```text
Agents' files: once a file has been fully made, this should always be the case so that no changes are ever lost.
```

<!-- workflow:excerpt C003 45 45 -->

[Transcript lines 45-45](../transcript.txt#L45-L45)

```text
It's very important that every stage emphasizes parallelism and avoiding blockers, and either has these subagents each write into a dedicated file or zeta files (such as their own file or any to_from file they make). Then they could all be in the same one but later if they start editing wide files, all of those must happen through a PR. That needs to be clear and part of the skill.
```

## Normalized purpose

Enforce strict single-writer file ownership where concepts own dedicated files and shared wide documents are modified only via pull requests.

## Initial thoughts

- Concurrent edits to shared wide files cause merge conflicts that drop information.
- Partitioning writers by file eliminates merge collisions during parallel research.

## Inferred suggestions

- Inference: Use atomic directory leases in agent-work/ to guarantee mutual exclusion on wide shared files.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
