# Reading long, rambling or garbled input

Read this in step 3. The aim is to lose nothing. Every idea, hint, constraint and change of mind ends up somewhere a later stage will see it. Judging whether an idea is good is for S2 and later.

## Passes

Read the whole input once before writing anything. Then make these passes over `segments.txt`, each writing to `$WORK/reading.md` with segment ids:

1. **Topics.** What would someone look up on its own? Each topic is a candidate concept.
2. **Directives.** How the user wants the work done: parallelism, PRs, branch rules, what to skip, where to stop. These go to `G` and also to every concept they constrain.
3. **Deliverables.** The files, documents, skills and issues the user expects to exist. Each one becomes a concept `target` (`kind: new`) or an edit of an existing file (`kind: edit`).
4. **Hints and gaps.** Ideas the user only gestured at ("the calendar thing, not going to explain it again"), and steps the user said they may have missed ("I may have missed steps"). Add the obvious missing pieces as `Inference:` suggestions.
5. **Contradictions and changes of mind.** Keep both statements. "Daily is fine, never hourly" after "hourly backups" is a change of mind. Unless the user set a different rule, the later statement is the working reading and the earlier one is kept with `supersedes`. If the user did set a rule ("later one wins, keep both"), record it in `G`.
6. **Repeats.** The same idea said twice is one concept citing both segments. Differences between the two tellings become thoughts.
7. **Garbled tokens.** Speech-to-text homophones, dropped words, stray symbols, typos. Each one goes in the register (below).

## How many concepts

- A sentence or a one-topic paragraph is **one concept**. It still gets the full package; there is no special layout.
- Make **one concept per document to create or file to edit**, or per topic a reader would research from different sources than its neighbours.
- **Split** when two parts would be researched from different sources, land in different files, or could be decided independently.
- **Merge** when two parts can't be researched or decided apart.
- Typical counts: one page gives 3 to 8 concepts; a multipage ramble gives 10 to 20. Above about 25, group the concepts with `part-of` relations under a few themes rather than dropping detail.
- A segment holding several ideas goes to every concept it feeds. Each concept's `summary` says which part it takes.

## Long input: parallel range readers

Above about 400 segments, or 60k characters, split the reading. Launch read-only subagents in one turn, each on a range of about 150 segments overlapping its neighbours by 5. Each one returns the seven lists above, with segment ids, and no prose summary. You merge and de-duplicate their lists. Then read every range boundary yourself, and spot-check three claims per reader against the text. You alone write the outline.

## Reading register (garbled input)

The transcript stays exact. Readings live only in the register in `main.md` ("Reading of garbled input") and in summaries labelled as inference. Each entry has these fields:

- the segment id;
- the exact token;
- the reading adopted;
- why;
- confidence (`high`, `medium` or `low`);
- the alternative readings considered.

Examples, all inference, from the request that started this workflow (`doc/2026-10-19T06-09-02Z_research_research-intake-to-implementation/transcript.txt`):

- `**§**` in "build the next many future **§**" is read as "features" (medium). The sentence lists things to implement, defer or reject.
- "zeta files" is read as "meta files" (high). The same sentence describes "to_from" files, which the request elsewhere calls meta files.
- "file_theyr file" is read as "file_their file", a directed note named after sender and receiver (high).
- "separate server agents" is read as "separate subagents" (medium). The alternative is "separate server-side agents".
- "minimal other contacts" is read as "minimal other context" (high).
- "ale skills" is read as "all skills" (medium). The alternative is "make skills".

A token whose reading would change what gets built (a product name, a number, a negation) also goes in the turn report as a Question with the default reading.
