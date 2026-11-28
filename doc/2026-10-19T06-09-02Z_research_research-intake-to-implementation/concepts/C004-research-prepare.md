# C004: Preliminary Research and Trigger Preparation

[Main reference](../main.md#c004)

## Relevant transcript excerpts

<!-- workflow:excerpt C004 21 25 -->

[Transcript lines 21-25](../transcript.txt#L21-L25)

```text
If there's more than one and you had to make a meta design doc (main reference document), then you'll now proceed to run a skill for each. Each file or concept from the main reference doc for this message, the skill will handle by preparing and doing preliminary research in order to hand off to a task that will generate issues and create an issue that will be used to trigger the full research for each of the individual aspects to investigate, or review, or confirm, or whatever is needed, in order to fully be able to understand the situation with minimal other contacts needed to be pulled in. If you only had one then you just run that skill against this and can skip many of the steps below.

This skill should not build out the whole research. It should prepare the beginning of the research and link to your core summary for the set, or the core summary for the set that you or another agent will write. It should link to its section in that document and to the raw transcript document and should have a section near the top that pulls all the relevant transcript portions, which are related to that specific document

The goal should be to prepare it for the next skill, which should perform a full set of research, gathering all details dedicated to a single one of these documents (although it can be aware of, link to, and reference other documents and details).
```

## Normalized purpose

Map repository context, codebase architecture, and prior art to draft research angles and self-contained trigger issues before deep investigation.

## Initial thoughts

- A lightweight exploration pass positions the research without flooding the context window.
- Trigger issues provide zero-context handoff briefs for dedicated research workers.

## Inferred suggestions

- Inference: Extract relevant transcript passages near the top of each concept stub to anchor investigation.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
