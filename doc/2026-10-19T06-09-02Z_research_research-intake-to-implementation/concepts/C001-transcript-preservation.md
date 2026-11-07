# C001: Exact Transcript Storage and Integrity Sealing

[Main reference](../main.md#c001)

## Relevant transcript excerpts

<!-- workflow:excerpt C001 1 5 -->

[Transcript lines 1-5](../transcript.txt#L1-L5)

```text
Please prepare to write a series of research and design docs. Your goal right now is not to make the documents, or even to build all the handoffs or do the full preliminary research. Your job will be to design a standard way to store and add more documentation and research planning and build a reliable method or skill for:
- taking a paragraph, a sentence, or many paragraphs
- storing the exact transcript somewhere
- breaking that down into a series of organized documents that are collected together, which are just organized, restated, cleaned-up versions of the information as needed at the top
- performing some basic supplemental details and exploration of the current code docs and architecture and the topics referenced in order to better position the research document
```

<!-- workflow:excerpt C001 7 7 -->

[Transcript lines 7-7](../transcript.txt#L7-L7)

```text
If there are enough details in the message for there to need to be multiple files made or edited, covering many different topics, then it should start with a skill that makes a main reference document. That document should list each new file to create or edit. The other agents will handle later. The main reference document should be dated in a standard location and will be the one which everything links into. It should link to the raw transcript of the request in another file if it's long. Either way it should break down the request into the major documents that need to be created or edited. Pre-create or touch all the files that need to be made, but not any meta files. Those can be made later. That's also defined later. This is just a single agent running the skill and the precreated files will be basically empty other than some basic header intro information and the most relevant excerpts from the raw input. Some basic framing and links to the main reference document and the raw transcript after it's made some number of basic files and done a first attempt at breaking down the request into a variety of files representing their concepts or purpose. It should have:
```

## Normalized purpose

Preserve the raw user prompt byte-for-byte in transcript.txt without normalization or whitespace changes, verifying its integrity via SHA-256.

## Initial thoughts

- Raw source must remain completely unedited regardless of typos or noisy speech-to-text.
- Tooling must refuse non-UTF-8 unless explicitly encoded, and prevent git hooks from altering trailing spaces.

## Inferred suggestions

- Inference: Use .gitattributes (-text) and .editorconfig to protect transcript.txt from formatting tools.

## Research Angles

1. **Sealed Transcript Protection Mechanisms**:
   - How `root.just`, `treefmt.toml`, `.gitattributes`, and `.editorconfig` protect byte-exact files against trailing-whitespace stripping and newline normalization across different platforms (Linux, macOS, Windows).
   - What additional hooks or CI checks are required to prevent accidental modifications to `transcript.txt`.
2. **Encoding & Non-UTF-8 Ingest**:
   - How non-UTF-8 byte sequences should be detected, flagged, and handled without silent data corruption.
   - Whether binary or non-text assets should be preserved alongside transcripts or referenced via content-addressed blob stores.
3. **Transcript Integrity Verification**:
   - Evaluating hash verification mechanisms (`expect-sha256`, pre-commit SHA checks, and git blob hash comparison).
   - How to verify that downstream concept excerpts always trace back to original source code-point offsets.
4. **Segmentation & Traceability Models**:
   - Evaluating sentence-level deterministic boundary detection (`T001`, `T002`, ...) vs line-based slicing when transcripts contain long single-line speech-to-text paragraphs.
5. **Garbled Input & Speech-to-Text Registration**:
   - Best practices for maintaining explicit transliteration registers without altering raw input text, drawing from phonetic alignment patterns in voice engineering.

## Codebase Context & References

- Tooling Scaffold: `scripts/research_workflow.py:scaffold` and `check_package`
- Protection Attributes: `.gitattributes` (`transcript.txt -text`) and `.editorconfig` (`trim_trailing_whitespace = false`)
- Workspace Scripts: `root.just` (`strip-whitespace` exclusion for `transcript.txt`)
- Upstream Cairn Request: `other/2026-10-19_research-design-workflow_request.txt`
- Workflow Specification: [`planning/workflow/records.md`](../../../planning/workflow/records.md)

## Trigger Issue Draft

```markdown
# Research: Exact Transcript Storage and Integrity Sealing (2026-10-19T06-09-02Z_research_research-intake-to-implementation·C001)

## Goal

Conduct deep factual research and architectural analysis for **Exact Transcript Storage and Integrity Sealing**, evaluating feasibility, prior art, use cases, risks, and concrete alternatives.

## Context Pointers

- Package Reference: [main.md](../main.md#c001)
- Concept Document: [C001-transcript-preservation.md](C001-transcript-preservation.md)
- Source Transcript: [transcript.txt](../transcript.txt) (Segments: L1-L5, L7)

## Scope & Questions

1. How do leading agent harnesses protect immutable source files from formatting hooks?
2. What are the failure modes of character-offset vs sentence-offset excerpt tracing?
3. What is the optimal error model for non-UTF-8 or corrupted transcripts?

## Expected Deliverables

1. Definitive concept research documented in `concepts/C001-transcript-preservation.md`.
2. Any cross-cutting discoveries sent as directed notes under `notes/`.
3. Clear alternative steelmanning and risk tables.

<!-- workflow:issue {"package": "2026-10-19T06-09-02Z_research_research-intake-to-implementation", "concept": "C001", "stage": "S1"} -->
```

## Stage results

Stage 1 (Prepare) completed: research angles, codebase context, and trigger issue drafted. Ready for Stage 2 (Concept Research).
