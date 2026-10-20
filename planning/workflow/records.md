# Workflow Records & Document Structure

This document specifies the internal file layout, marker syntax, state blocks, and formatting rules for research collections.

## 1. Research Collection Layout (`doc/<UTC>_research_<slug>/`)

Each research collection is housed in a dedicated directory under the configured `requests_root`:

```text
doc/<UTC>_research_<slug>/
├── transcript.txt                  # Exact immutable source bytes (protected from formatters)
├── package.json                    # Collection manifest (transcript metadata, segment list)
├── main.md                         # Main reference document (overview, concept sections, issue index)
├── design.md                       # Meta design (review order, waves, run status table, S5 stages)
├── changes.md                      # Append-only change log (merge=union)
├── concepts/
│   ├── C001-<slug>.md              # Owned concept document (contains concept state block)
│   └── C002-<slug>.md
├── notes/                          # Directed cross-agent notes (created lazily)
│   └── C001__to__C002__<uuid>.md
├── receipts/                       # Note resolution receipts (created atomically by receivers)
│   └── C001__to__C002__<uuid>.md
└── decisions/                      # Structured decision records (created at S6)
    └── D001-<slug>.md
```

## 2. Formatting Stability & Parser Immunity

To prevent code formatters (`treefmt`, `prettier`, `biome`) from corrupting machine-readable metadata:

- **State Blocks**: Stored within fenced code blocks or delimited comments:

  ```markdown
  <!-- workflow:state -->

  {
  "version": 1,
  "id": "2026-10-19T06-09-02Z_research_intake",
  "stage": "intake",
  "status": "prepared"
  }

  <!-- /workflow:state -->
  ```

  Prettier preserves JSON syntax inside these blocks.

- **Source Excerpts**: Placed inside dedicated excerpt blocks with a double newline following the header comment:

  ````markdown
  <!-- workflow:excerpt C001 1 5 -->

  [Transcript lines 1-5](../transcript.txt#L1-L5)

  ```text
  Exact excerpt text preserved byte-for-byte.
  ```
  ````

  ```

  ```

- **Transcript Protection**: `transcript.txt` is marked `-text` in `.gitattributes` and excluded from trailing whitespace fixers in `root.just`.

## 3. Identification & Traceability

- **Segments (`T001`, `T002`, ...)**: Punctuation and sentence-bounded units of the raw transcript, assigned sequentially at intake.
- **Concepts (`C001`, `C002`, ...)**: Major topics or requirements extracted from the transcript.
- **Hierarchical Items (`C001.1`, `C001.2.1`, ...)**: Granular statements, findings, and design requirements that evolve over time.
- **Status Trails**: When an item is clarified, superseded, or retracted, the new status is prepended with a UTC timestamp and originating stage, preserving historical provenance:
  ```markdown
  - **C001.2** `clarified` · S2 · 2026-10-21T10:00:00Z — Refined interpretation. ← `created` · S0 · 2026-10-19T06:09:02Z
    - **C001.2.1** `created` · S2 · 2026-10-21T10:00:00Z — Specific child detail.
  ```
- **Garbled Input Register**: Ambiguous phonetic words or speech-to-text artifacts are documented in a dedicated register in `main.md` (e.g. `Karen` -> `Cairn`, `§` -> `features`), keeping interpretations explicit and audit-traceable.
