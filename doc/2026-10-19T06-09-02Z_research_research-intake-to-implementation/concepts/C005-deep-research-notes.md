# C005: Deep Factual Research and Cross-Agent Notes

[Main reference](../main.md#c005)

## Relevant transcript excerpts

<!-- workflow:excerpt C005 27 31 -->

[Transcript lines 27-31](../transcript.txt#L27-L31)

```text
This stage will have many agents running it so if an agent wants to change something in the file other than its own file, it should write to a file named something like that file_theyr file or something similar. Either the other agent might send something to their file_that file or will simply incorporate that change or notification into their document.

Theoretically if there were three documents and each one talked to the other ones, there would be six of these documents, two for each. The primary goal for this is to fully detail the request, feasibility, and research information online in detail, covering all facets and including any and all links and references that were used.

This research can include user anecdotes, example projects which use a similar setup, or any other details it feels prudent to include. There's no limit or restriction on the amount of detail that can be put in this and over time more detail will be added. It could be considered the definitive document for the concept or purpose it's working at and should likely have a change log, either as a separate file or at the bottom or top, marking major changes, if not every little minor fix or notice.
```

## Normalized purpose

Conduct deep factual research per concept, steelmanning alternatives and exchanging cross-cutting findings via immutable directed notes.

## Initial thoughts

- Research workers operate in parallel on isolated concept documents.
- Workers never edit files owned by other agents; they write immutable notes to notes/.

## Inferred suggestions

- Inference: Seal directed notes with SHA-256 body hashes to enforce immutability.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
