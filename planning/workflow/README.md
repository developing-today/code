# Research and Delivery Workflow

A portable, evidence-preserving framework for transforming complex, rambling, or voice-transcribed requests into structured research, aligned designs, granular implementation plans, and parallel pull-request deliveries.

## Core Philosophy

1. **Lossless Intake**: The original transcript is stored byte-for-byte and sealed with its SHA-256 hash. Typos, phonetic speech-to-text ambiguities, and tangents are preserved exactly. Interpretations and normalizations are separated from raw source text.
2. **One Writer per File**: Every document and note has exactly one owning writer at a time. Multi-agent collaboration proceeds through append-only directed notes, dedicated receipts, and pull requests rather than concurrent edits to shared files.
3. **Thin Orchestrators**: Orchestrating agents coordinate, dispatch, monitor merge states, and restart stalled workers. They never write implementation code.
4. **Typed Dependency Scheduling**: Edits and tasks are organized as a dependency graph that distinguishes hard blockers (`requires`) from soft sequencing (`recommended-after`) and concurrency fences (`shares-files-with`).
5. **Decisions with Rationale**: Conflicts trigger independent reviewer and critic subagents that make reasoned preliminary decisions, keeping work moving while preserving a minimal list of consequential choices for human review.

## Architecture

| Guide                             | Description                                                                                           |
| --------------------------------- | ----------------------------------------------------------------------------------------------------- |
| [Stages](stages.md)               | The 9 lifecycle stages (S0 intake through S8 implement) and the router skill.                         |
| [Records](records.md)             | Package structure, transcript immutability, concept state blocks, item trails, and note formats.      |
| [Collaboration](collaboration.md) | The 10 collaboration rules, worktree boot protocol, branch auto-delete handling, and lock management. |
| [Decisions](decisions.md)         | Decision records, preliminary choices, challenge passes, and human review lists.                      |
| [Ordering](ordering.md)           | Hard vs. soft edges, wave calculations, ready frontier dispatching, and priority tiers.               |
| [Profile](profile.md)             | Repository adapter specification (`.agents/research-workflow.json`).                                  |
| [Templates](templates/)           | Canonical templates for concepts, notes, receipts, decisions, and issues.                             |
