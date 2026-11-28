# Documentation Architecture

This document defines where documentation lives in this repository, what belongs at the repository root versus subdirectories, when a topic deserves its own folder, and how maintained documents relate to immutable records.

## 1. Documentation Hierarchy

```text
/
├── README.md                          # Top-level human orientation
├── ROADMAP.md                         # Cross-project roadmap & horizons
├── AGENTS.md                          # Always-loaded agent rulebook (compact)
├── CLAUDE.md                          # Claude Code configuration & environment guide
├── doc/
│   ├── README.md                      # Complete documentation catalog & topic map
│   ├── DOCUMENTATION_PROTOCOL.md      # Rules for dated records & maintained files
│   ├── references/                    # Shared external standards & reference files
│   └── <UTC>_<kind>_<slug>/           # Dated immutable records & research packages
├── planning/
│   ├── README.md                      # Three-level planning framework
│   ├── DOCUMENTATION_ARCHITECTURE.md  # This document
│   ├── DEPENDENCIES.md                # Dependency kinds & relation taxonomy
│   ├── roadmaps/                      # Per-component roadmaps (Now / Next / Later / Ideas)
│   ├── designs/                       # Pointer to doc/ records
│   ├── plans/                         # Granular implementation plans
│   └── workflow/                      # Staged research and delivery workflow library
└── pkgs/<component>/                  # Living component-owned documentation
```

## 2. Placement Rules

| Location                       | Purpose                                                                                                               | Mutability                                                                    |
| ------------------------------ | --------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| **Repository root**            | Entry points, high-level portfolio roadmap, and core agent steering. Kept concise to save context.                    | Maintained in place                                                           |
| **`doc/<UTC>_<kind>_<slug>/`** | Point-in-time design, architecture, and feature records, plus research collections (`<UTC>_research_<slug>/`).        | Immutable records; updates append new sections or link to superseding records |
| **`doc/README.md`**            | Master topic index and navigation map for all documentation across the repository.                                    | Maintained catalog                                                            |
| **`doc/references/`**          | Long-lived external standards, protocol specifications, and shared research papers.                                   | Append-only / updated on new releases                                         |
| **`planning/roadmaps/`**       | Project-level horizon tracking (Now, Next, Later, Ideas).                                                             | Maintained in place                                                           |
| **`planning/plans/`**          | Step-by-step implementation plans broken into pull-request sized layers.                                              | Updated as steps land                                                         |
| **`planning/workflow/`**       | Specification of the 9-stage research-to-delivery lifecycle, templates, and collaboration rules.                      | Versioned framework library                                                   |
| **`pkgs/<component>/`**        | Deep module documentation, API references, architecture guides, and internal design decisions owned by that codebase. | Maintained with code changes                                                  |

## 3. When a Topic Deserves a Dedicated Folder

Create a dedicated folder under `doc/` when:

1. **A new feature or architectural change** alters system behavior, protocols, storage formats, or public APIs (use `<UTC>_<kind>_<slug>/`).
2. **A multi-topic research request** arrives requiring exact transcript preservation, concept decomposition, and parallel investigation (use `<UTC>_research_<slug>/`).
3. **Investigation artifacts** include benchmark logs, test harnesses, diagrams, or multiple supporting notes alongside the primary document.

Keep as a standalone file when:

- It is a single-topic implementation plan belonging in `planning/plans/<slug>.md`.
- It is a living reference document inside `planning/` or `pkgs/<component>/`.

## 4. Immutability vs. Maintained Documents

- **Dated Records (`doc/<UTC>_...`)**: Once closed, historical records represent what was known and decided at that point in time. They are never rewritten. If a later stage clarifies or reverses a decision, it records a new document or updates a maintained main reference, linking back to the original record.
- **Maintained Documents (`planning/`, `AGENTS.md`, `ROADMAP.md`, `doc/README.md`)**: Updated continuously as work progresses, with changes committed via reviewed pull requests.
