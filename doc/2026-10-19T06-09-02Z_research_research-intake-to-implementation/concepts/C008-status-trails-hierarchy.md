# C008: Hierarchical Items, Status Trails, and Change History

[Main reference](../main.md#c008)

## Relevant transcript excerpts

<!-- workflow:excerpt C008 39 41 -->

[Transcript lines 39-41](../transcript.txt#L39-L41)

```text
The main reference doc should mark which stage each difference came from and when sections were superseded, clarified, or retracted by a later stage. The status section of that piece of information should be updated, listing the new status in front and the date/time. If it was at stage "created" you'd put "stage revoked" or "clarified" afterwards with the detail, a short sentence explaining what happened. It could just be "clarified" and then you say, "The situation is more nuanced than we thought." Please refer to the below item, whatever.whatever.whatever, which will likely be immediately below anything from the given stage.

I imagine that the sections will be hierarchical, starting maybe with each file or concept, and then there's a sub-item with some of the details and initial thoughts. In summaries PRs and issues should be attached in the reference doc, inline with the file section. At the bottom there should be an "Issues and PRs" section, which lists it again and references what file or subsection of a file it refers to specifically. It should also be referenced in the meta design doc.
```

## Normalized purpose

Maintain granular hierarchical items (C001.1, C001.2.1) and prepend status trails (created -> clarified -> confirmed) to track evolution over time.

## Initial thoughts

- Information is clarified or superseded rather than erased.
- Status changes record timestamp, stage, and rationale with backlinks to refined items.

## Inferred suggestions

- Inference: Support dotted concept and item IDs in the tool CLI and validation regex.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
