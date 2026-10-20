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

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
