# C002: Main Reference and Review Order Meta Design

[Main reference](../main.md#c002)

## Relevant transcript excerpts

<!-- workflow:excerpt C002 8 12 -->

[Transcript lines 8-12](../transcript.txt#L8-L12)

```text
- A list of each different file in the main reference document.
- An introduction.
- Some preliminary thoughts for each item.
- An empty placeholder item for the summary results of the next skill sections for related and created issues and PRs, their active status, and reasons they were implemented, declined, or fundamentally changed.
- A related change file and a design document which takes the first pass at the appropriate order to review each different file or concept.
```

<!-- workflow:excerpt C002 15 15 -->

[Transcript lines 15-15](../transcript.txt#L15-L15)

```text
If none of them block any of the others or there's nothing particularly important, then the plan will just say: run one agent for each of these, enlist each of them, and include the current status of whether or not it's been run. Later jobs will add the GitHub issues and PRs and the design docs created later on related to item.
```

<!-- workflow:excerpt C002 17 17 -->

[Transcript lines 17-17](../transcript.txt#L17-L17)

```text
This meta design and the main reference document will be linked in many places and should be kept up to date, which is why you need a change log after this initial creation. Other changes should all be done using pull requests to ensure that conflicts are handled and no one overwrites another.
```

## Normalized purpose

Generate main.md as the primary collection anchor with per-concept sections, preliminary thoughts, and design.md specifying initial review order and waves.

## Initial thoughts

- main.md acts as the central link target for all downstream artifacts.
- design.md proposes an initial sequencing order for reviewing concepts, separating blockers from parallel paths.

## Inferred suggestions

- Inference: Render machine state into fenced blocks rather than fragile HTML comments to survive prettier.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
