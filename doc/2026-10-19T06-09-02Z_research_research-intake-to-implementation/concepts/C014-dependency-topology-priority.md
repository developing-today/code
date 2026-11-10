# C014: Dependency Topology, Hard Blockers, and Priority Tiers

[Main reference](../main.md#c014)

## Relevant transcript excerpts

<!-- workflow:excerpt C014 73 73 -->

[Transcript lines 73-73](../transcript.txt#L73-L73)

```text
were going to build the next many future **§** to be implemented. maybe some will be deferred indefinitely, others will be eventually rejected, but most all will certainly be implemented. we'll need to have an organized plan that defines the appropriate order to implement and which can be, shouldn't be or must be implemented before or after others so we can be confident in building many features at once. be clear between hard blockers and which things could technically be done before or at same time but will be more difficult or needed rewrite once the rest comes in. which are absolutely crucial, which are nice to have or recommended and which are there in case we one day need to take that path. what is the end goals, what does each bring to us and enable.
```

## Normalized purpose

Distinguish hard blockers (requires) from soft sequencing (recommended-after), concurrency fences, and priority tiers (crucial, recommended, contingent).

## Initial thoughts

- Soft sequencing edges document rework risks without artificially blocking parallel workers.
- Contingent paths carry explicit activation triggers and are deferred until needed.

## Inferred suggestions

- Inference: Validate dependency acyclicity and enforce wave partitioning in the ready frontier calculation.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
