# Platform clients: SSH modes, rendering, transport and native clients

This package captures the 2026-10-18 platform world state report (829 lines, 48,593 characters, 703 non-blank lines) as a request for research on the `id` platform's clients: SSH input and presentation modes, terminal rendering, one command grammar, transport, and native clients. Eleven concepts follow the concept list the requesting session gave. A twelfth, C012, stands in for the global bucket G, because the workflow tool on main has no G. PR #211, which the report calls open, merged on 2026-10-20, so S1 starts by re-checking each claim against main. Intake stopped at S0, and no research has run.

<!-- workflow:state -->

{
"version": 1,
"id": "2026-10-22T11-10-22Z_research_platform-clients",
"created_at": "2026-10-22T11:10:22Z",
"stage": "intake",
"status": "prepared",
"transcript": {
"path": "transcript.txt",
"sha256": "409caeb8e4542dd50a22ef4e0186a6d599c0678115b15b2d414e44338d28df62"
},
"concepts": [
{
"id": "C001",
"title": "SSH command (line) mode and the #211 gaps",
"path": "concepts/C001-ssh-line-mode.md",
"status": "created",
"source_lines": [
[
29,
44
],
[
46,
48
],
[
53,
55
],
[
130,
146
],
[
178,
189
],
[
194,
253
],
[
262,
277
],
[
317,
337
],
[
353,
379
],
[
381,
393
],
[
399,
454
],
[
492,
492
],
[
505,
505
],
[
571,
571
],
[
629,
631
],
[
641,
648
],
[
748,
749
],
[
779,
785
]
],
"priority": "crucial",
"horizon": "now",
"type": "topic",
"kind": "edit",
"targets": [
"pkgs/id/src/world_ssh.rs",
"doc/2026-10-07T00-20-00Z_architecture_roc_worlds/2026-10-07T00-20-00Z_architecture_roc_worlds.md",
".opencode/plans/phase-11-foundations.md"
],
"goal": "Every SSH line-mode path dispatches through the session driver with bounded output and no silent input loss, and a committed doc records the username grammar.",
"issues": [
{
"number": 548,
"url": "https://github.com/developing-today/code/issues/548",
"stage": "S0",
"state": "open",
"title": "id(ssh): fix #211 reconciliation gaps: total records cap, document output through `send`, per-world document store",
"reason": "The #211 gaps: total records cap, document output through send, per-world store (D1-039)."
},
{
"number": 549,
"url": "https://github.com/developing-today/code/issues/549",
"stage": "S0",
"state": "open",
"title": "id(ssh): SSH input queue silently drops keystrokes when full",
"reason": "Full input queue drops keystrokes silently (D1-058)."
}
],
"prs": []
},
{
"id": "C002",
"title": "In-session command toggle and split UI",
"path": "concepts/C002-in-session-toggle-split-ui.md",
"status": "created",
"source_lines": [
[
38,
40
],
[
44,
44
],
[
173,
176
],
[
259,
260
],
[
294,
302
],
[
311,
316
],
[
325,
332
],
[
335,
337
],
[
345,
347
],
[
373,
374
],
[
389,
389
],
[
456,
462
],
[
657,
657
],
[
788,
788
]
],
"priority": "contingent",
"horizon": "later",
"type": "topic",
"kind": "edit",
"targets": [
"pkgs/id/src/world_ssh.rs"
],
"goal": "A player can issue commands mid-game without leaving the world screen, with a visible mode and no mis-routed input.",
"issues": [
{
"number": 535,
"url": "https://github.com/developing-today/code/issues/535",
"stage": "S0",
"state": "open",
"title": "id(ssh): input and presentation designs",
"reason": "Tracker items D1-045 (in-session toggle) and D1-046 (split UI)."
}
],
"prs": []
},
{
"id": "C003",
"title": "Alternate screen, incremental rendering and redraw batching",
"path": "concepts/C003-terminal-rendering.md",
"status": "created",
"source_lines": [
[
38,
40
],
[
157,
172
],
[
254,
258
],
[
279,
292
],
[
339,
344
],
[
369,
370
],
[
518,
519
],
[
544,
544
],
[
555,
556
],
[
632,
634
],
[
705,
708
],
[
743,
747
],
[
751,
752
]
],
"priority": "recommended",
"horizon": "next",
"type": "topic",
"kind": "edit",
"targets": [
"pkgs/id/src/world_presentation.rs",
"pkgs/id/src/world_ssh.rs",
"pkgs/id/src/world_session.rs"
],
"goal": "Interactive sessions repaint only what changed, on the alternate screen, at a bounded frame rate, without filling scrollback.",
"issues": [
{
"number": 545,
"url": "https://github.com/developing-today/code/issues/545",
"stage": "S0",
"state": "open",
"title": "id(ssh): use the alternate screen for raw interactive SSH sessions",
"reason": "Alternate screen for raw interactive sessions (D1-042)."
},
{
"number": 561,
"url": "https://github.com/developing-today/code/issues/561",
"stage": "S0",
"state": "open",
"title": "id: server-side redraw batching (few-ms frame coalescing) for all presentations",
"reason": "Server-side redraw batching (D1-043)."
},
{
"number": 562,
"url": "https://github.com/developing-today/code/issues/562",
"stage": "S0",
"state": "open",
"title": "id(ssh): incremental (diff) ANSI renderer instead of full clear-repaint",
"reason": "Incremental diff ANSI renderer (D1-044)."
},
{
"number": 536,
"url": "https://github.com/developing-today/code/issues/536",
"stage": "S0",
"state": "open",
"title": "id(transport): transport designs",
"reason": "Tracker item D1-029, F12.11 delta and binary frames, the protocol counterpart of the diff renderer."
}
],
"prs": []
},
{
"id": "C004",
"title": "Presentation capability tiers and the kitty keyboard protocol",
"path": "concepts/C004-presentation-tiers.md",
"status": "created",
"source_lines": [
[
107,
110
],
[
333,
333
],
[
393,
393
],
[
728,
752
],
[
792,
793
]
],
"priority": "recommended",
"horizon": "later",
"type": "topic",
"kind": "edit",
"targets": [
"pkgs/id/src/world_ssh.rs",
"pkgs/id/examples/roc-world/Key.roc"
],
"goal": "Ghostty-class terminals get the full experience and every other terminal degrades predictably, detected in-band over any transport.",
"issues": [
{
"number": 535,
"url": "https://github.com/developing-today/code/issues/535",
"stage": "S0",
"state": "open",
"title": "id(ssh): input and presentation designs",
"reason": "Tracker items D1-052 (presentation tiers) and D1-053 (kitty keyboard on raw input)."
}
],
"prs": []
},
{
"id": "C005",
"title": "Documents as a session frame pair",
"path": "concepts/C005-documents-frame-pair.md",
"status": "created",
"source_lines": [
[
203,
206
],
[
234,
234
],
[
243,
246
],
[
388,
388
],
[
406,
418
],
[
432,
432
],
[
444,
444
],
[
658,
658
],
[
786,
787
]
],
"priority": "recommended",
"horizon": "later",
"type": "topic",
"kind": "edit",
"targets": [
"pkgs/id/src/world_session.rs",
"pkgs/id/src/world_ssh.rs",
"pkgs/id/web/src/world.ts"
],
"goal": "Web, CLI and SSH clients read a world's documents through one authorized protocol path.",
"issues": [
{
"number": 535,
"url": "https://github.com/developing-today/code/issues/535",
"stage": "S0",
"state": "open",
"title": "id(ssh): input and presentation designs",
"reason": "Tracker item D1-040 (documents as a session frame pair)."
},
{
"number": 548,
"url": "https://github.com/developing-today/code/issues/548",
"stage": "S0",
"state": "open",
"title": "id(ssh): fix #211 reconciliation gaps: total records cap, document output through `send`, per-world document store",
"reason": "Prerequisite gaps: document output through send and a per-world document store."
}
],
"prs": []
},
{
"id": "C006",
"title": "One command grammar and --json output",
"path": "concepts/C006-command-grammar-json.md",
"status": "created",
"source_lines": [
[
36,
38
],
[
182,
183
],
[
186,
189
],
[
207,
209
],
[
238,
240
],
[
319,
321
],
[
371,
372
],
[
375,
376
],
[
386,
386
],
[
390,
392
],
[
395,
397
],
[
422,
426
],
[
433,
433
],
[
463,
464
],
[
470,
475
],
[
629,
631
],
[
657,
658
],
[
784,
785
]
],
"priority": "crucial",
"horizon": "next",
"type": "topic",
"kind": "edit",
"targets": [
"pkgs/id/src/world_ssh.rs",
"pkgs/id/src/commands/world.rs",
"pkgs/id/src/cli.rs"
],
"goal": "The same command sentence works as an SSH line, a piped script line and a CLI one-off, with deterministic machine-readable output.",
"issues": [
{
"number": 546,
"url": "https://github.com/developing-today/code/issues/546",
"stage": "S0",
"state": "open",
"title": "id: one line grammar for SSH command mode, the `id world join` pump and CLI verbs (`--command-mode`)",
"reason": "One line grammar and --command-mode (D1-041)."
},
{
"number": 563,
"url": "https://github.com/developing-today/code/issues/563",
"stage": "S0",
"state": "open",
"title": "id: `--json` transcript output in world line mode",
"reason": "--json transcript output in line mode, after #546."
}
],
"prs": []
},
{
"id": "C007",
"title": "SSH as a frame transport and SSH-over-iroh",
"path": "concepts/C007-ssh-transport.md",
"status": "created",
"source_lines": [
[
190,
192
],
[
259,
260
],
[
303,
310
],
[
378,
379
],
[
467,
468
],
[
662,
727
],
[
772,
773
],
[
789,
791
]
],
"priority": "contingent",
"horizon": "later",
"type": "topic",
"kind": "edit",
"targets": [
"pkgs/id/src/world_ssh.rs",
"pkgs/id/src/commands/world.rs"
],
"goal": "Clients can reach worlds over SSH where QUIC cannot go, and third-party SSH clients can reach NAT'd hosts, without SSH becoming the default.",
"issues": [
{
"number": 536,
"url": "https://github.com/developing-today/code/issues/536",
"stage": "S0",
"state": "open",
"title": "id(transport): transport designs",
"reason": "Tracker items D1-050 (SSH as a frame transport) and D1-051 (SSH-over-iroh)."
},
{
"number": 564,
"url": "https://github.com/developing-today/code/issues/564",
"stage": "S0",
"state": "open",
"title": "id: write the Phase 13 plan: client shells and client capability providers",
"reason": "The Phase 13 plan holds D1-050 and D1-051."
}
],
"prs": []
},
{
"id": "C008",
"title": "Native client shells",
"path": "concepts/C008-native-client-shells.md",
"status": "created",
"source_lines": [
[
101,
102
],
[
541,
541
],
[
552,
553
],
[
754,
762
],
[
774,
777
]
],
"priority": "recommended",
"horizon": "later",
"type": "topic",
"kind": "new",
"targets": [
".opencode/plans/phase-13-client-shells.md (name set by #564)"
],
"goal": "A native shell can replace the terminal or browser by implementing the screen/input contract and passing conformance.",
"issues": [
{
"number": 538,
"url": "https://github.com/developing-today/code/issues/538",
"stage": "S0",
"state": "open",
"title": "id: native client shells and client capabilities",
"reason": "Tracker item D1-054 (native client shells)."
},
{
"number": 564,
"url": "https://github.com/developing-today/code/issues/564",
"stage": "S0",
"state": "open",
"title": "id: write the Phase 13 plan: client shells and client capability providers",
"reason": "The Phase 13 plan holds D1-054."
}
],
"prs": []
},
{
"id": "C009",
"title": "Client-side capability providers",
"path": "concepts/C009-client-capability-providers.md",
"status": "created",
"source_lines": [
[
540,
541
],
[
544,
545
],
[
548,
549
],
[
754,
756
],
[
763,
767
],
[
774,
777
]
],
"priority": "recommended",
"horizon": "later",
"type": "topic",
"kind": "new",
"targets": [
".opencode/plans/phase-13-client-shells.md (name set by #564)"
],
"goal": "Apps can use client-local files, GPU and network only through named, granted, ceilinged client providers.",
"issues": [
{
"number": 538,
"url": "https://github.com/developing-today/code/issues/538",
"stage": "S0",
"state": "open",
"title": "id: native client shells and client capabilities",
"reason": "Tracker item D1-055 (client-side capability providers)."
},
{
"number": 564,
"url": "https://github.com/developing-today/code/issues/564",
"stage": "S0",
"state": "open",
"title": "id: write the Phase 13 plan: client shells and client capability providers",
"reason": "The Phase 13 plan holds D1-055."
}
],
"prs": []
},
{
"id": "C010",
"title": "Multi-server and multi-client topology",
"path": "concepts/C010-multi-server-multi-client.md",
"status": "created",
"source_lines": [
[
556,
557
],
[
681,
687
],
[
754,
756
],
[
768,
777
]
],
"priority": "recommended",
"horizon": "later",
"type": "topic",
"kind": "new",
"targets": [
".opencode/plans/phase-13-client-shells.md (name set by #564)"
],
"goal": "One id client can join many worlds and servers at once and act as a server too, while client-only mode stays supported.",
"issues": [
{
"number": 536,
"url": "https://github.com/developing-today/code/issues/536",
"stage": "S0",
"state": "open",
"title": "id(transport): transport designs",
"reason": "Tracker item D1-056 (multi-server, multi-client topology)."
},
{
"number": 564,
"url": "https://github.com/developing-today/code/issues/564",
"stage": "S0",
"state": "open",
"title": "id: write the Phase 13 plan: client shells and client capability providers",
"reason": "The Phase 13 plan holds D1-056."
}
],
"prs": []
},
{
"id": "C011",
"title": "Roc partial-line surface",
"path": "concepts/C011-roc-partial-line.md",
"status": "created",
"source_lines": [
[
107,
110
],
[
142,
155
],
[
271,
274
],
[
345,
351
],
[
375,
376
],
[
443,
446
],
[
465,
466
],
[
658,
658
]
],
"priority": "contingent",
"horizon": "later",
"type": "topic",
"kind": "edit",
"targets": [
"pkgs/id/examples/roc-world/platform.roc",
"pkgs/id/src/sandbox.rs",
"pkgs/id/src/world_session.rs"
],
"goal": "A Roc app can react to what the viewer is typing without the partial line ever reaching the journal.",
"issues": [
{
"number": 535,
"url": "https://github.com/developing-today/code/issues/535",
"stage": "S0",
"state": "open",
"title": "id(ssh): input and presentation designs",
"reason": "Tracker item D1-048 (Roc partial-line surface)."
}
],
"prs": []
},
{
"id": "C012",
"title": "G: global directives and report context",
"path": "concepts/C012-global-context.md",
"status": "created",
"source_lines": [
[
1,
28
],
[
38,
41
],
[
45,
45
],
[
49,
128
],
[
358,
361
],
[
378,
379
],
[
477,
563
],
[
565,
615
],
[
617,
637
],
[
639,
660
],
[
779,
780
],
[
794,
829
]
],
"priority": "unspecified",
"horizon": "unspecified",
"type": "directive",
"kind": "edit",
"targets": [],
"goal": "Keep the report's framing, constraints and out-of-scope work visible to every concept without researching it here.",
"issues": [],
"prs": []
}
],
"dependencies": [
{
"from": "C002",
"to": "C001",
"kind": "optional-if",
"reason": "The toggle and the split UI switch between raw play and the line mode, so they extend it.",
"trigger": "Users ask to switch mode mid-session (L373-L374, L458-L460), or a concrete demand for a split UI (L461-L462)."
},
{
"from": "C002",
"to": "C006",
"kind": "recommended-after",
"reason": "The command buffer the toggle opens speaks the shared grammar (#535 D1-045 after #546).",
"rework_if_early": "The toggle prompt's command set diverges from the grammar and has to be redone."
},
{
"from": "C002",
"to": "C003",
"kind": "recommended-after",
"reason": "The split UI's pinned command row must survive world redraws (L299-L300; #535 D1-046 after #562).",
"rework_if_early": "A pinned row designed against full clear-repaint is redone for the diff renderer."
},
{
"from": "C003",
"to": "C002",
"kind": "enables",
"reason": "In-place, incremental redraw is what lets a world redraw without clobbering a command row (L299-L300)."
},
{
"from": "C003",
"to": "C004",
"kind": "enables",
"reason": "Every tier benefits from batching, and tier 2 depends on the diff renderer (L743-L747, L751)."
},
{
"from": "C004",
"to": "C003",
"kind": "recommended-after",
"reason": "Tier design builds on the alternate screen, batching and the diff renderer (#535 D1-052 after #545, #561, #562).",
"rework_if_early": "Tier-2 behaviour is specified against a renderer that then changes."
},
{
"from": "C001",
"to": "C004",
"kind": "enables",
"reason": "The line mode is the tier-3 degraded presentation (L748-L749)."
},
{
"from": "C005",
"to": "C001",
"kind": "recommended-after",
"reason": "The frame pair replaces the SSH-local document path whose gaps #548 fixes first (#535 D1-040 after #548).",
"rework_if_early": "The frame pair is designed before document output goes through send and the per-world store is decided."
},
{
"from": "C006",
"to": "C011",
"kind": "enables",
"reason": "The partial-line surface waits until the command grammar is known (L375-L376)."
},
{
"from": "C011",
"to": "C006",
"kind": "recommended-after",
"reason": "Deferred until the line mode exists and the grammar is known (L375-L376).",
"rework_if_early": "The partial-line event is shaped around commands that the grammar later renames."
},
{
"from": "C011",
"to": "C002",
"kind": "optional-if",
"reason": "The partial line lives in the split UI's command region (L345-L351; #535 D1-048 after D1-046).",
"trigger": "A split UI or toggle exists and a Roc app wants live autocomplete or a validating form (L348-L350)."
},
{
"from": "C007",
"to": "C001",
"kind": "optional-if",
"reason": "SSH as a frame transport and a second SSH channel attach to the same SSH server, at exec_request and subsystem_request (L190-L192, L467-L468).",
"trigger": "Constrained networks or jump hosts need the frame protocol over SSH (L690-L691), UDP is blocked (L716-L717), or a real client exists for two channels (L378-L379)."
},
{
"from": "C007",
"to": "C010",
"kind": "enables",
"reason": "SSH-over-iroh and SSH as a transport widen which clients reach which servers (L714-L719), which the multi-server topology uses (L768-L771)."
}
],
"issues": [],
"prs": [],
"tasks": [],
"events": [
{
"at": "2026-10-22T11:10:22Z",
"stage": "intake",
"status": "prepared",
"reason": "Exact transcript captured and initial concept files prepared."
}
]
}
<!-- /workflow:state -->

## Files

- [main.md](main.md)
- [transcript.txt](transcript.txt)
- [design.md](design.md)
- [changes.md](changes.md)
- [concepts/C001-ssh-line-mode.md](concepts/C001-ssh-line-mode.md)
- [concepts/C002-in-session-toggle-split-ui.md](concepts/C002-in-session-toggle-split-ui.md)
- [concepts/C003-terminal-rendering.md](concepts/C003-terminal-rendering.md)
- [concepts/C004-presentation-tiers.md](concepts/C004-presentation-tiers.md)
- [concepts/C005-documents-frame-pair.md](concepts/C005-documents-frame-pair.md)
- [concepts/C006-command-grammar-json.md](concepts/C006-command-grammar-json.md)
- [concepts/C007-ssh-transport.md](concepts/C007-ssh-transport.md)
- [concepts/C008-native-client-shells.md](concepts/C008-native-client-shells.md)
- [concepts/C009-client-capability-providers.md](concepts/C009-client-capability-providers.md)
- [concepts/C010-multi-server-multi-client.md](concepts/C010-multi-server-multi-client.md)
- [concepts/C011-roc-partial-line.md](concepts/C011-roc-partial-line.md)
- [concepts/C012-global-context.md](concepts/C012-global-context.md)

Transcript SHA-256: `409caeb8e4542dd50a22ef4e0186a6d599c0678115b15b2d414e44338d28df62`. Source bytes are immutable.

<a id="c001"></a>

## C001: SSH command (line) mode and the #211 gaps

Document: [concepts/C001-ssh-line-mode.md](concepts/C001-ssh-line-mode.md).

Takes the input decision (raw default, line mode as a second explicit entry chosen at login), how input flows today, what PR #211 added, its conflicts and known gaps, the mode use cases and §7 steps 1 and 2. #211 merged on 2026-10-20 after the report, so this concept now covers checking what shipped and closing the remaining gaps.

<!-- workflow:view concept-C001 -->

- **State:** `created` · stage S0 · priority `crucial` · horizon `now`
- **Goal:** Every SSH line-mode path dispatches through the session driver with bounded output and no silent input loss, and a committed doc records the username grammar.
- **Items:** none yet
- **Relations:**
  - `enables` [C004](#c004): The line mode is the tier-3 degraded presentation (L748-L749).
- **Referenced by:** [C002](#c002) `optional-if`, [C005](#c005) `recommended-after`, [C007](#c007) `optional-if`
- **Issues and PRs:**
  - Issue [#548](https://github.com/developing-today/code/issues/548) · S0 · `open` — id(ssh): fix #211 reconciliation gaps: total records cap, document output through \`send\`, per-world document store; The #211 gaps: total records cap, document output through send, per-world store (D1-039).
  - Issue [#549](https://github.com/developing-today/code/issues/549) · S0 · `open` — id(ssh): SSH input queue silently drops keystrokes when full; Full input queue drops keystrokes silently (D1-058).

<!-- /workflow:view concept-C001 -->

<a id="c002"></a>

## C002: In-session command toggle and split UI

Document: [concepts/C002-in-session-toggle-split-ui.md](concepts/C002-in-session-toggle-split-ui.md).

Takes the two in-session dual-channel shapes: a prefix-key toggle into a command buffer with a visible mode indicator (shape 3) and a split UI with a pinned command row and focus routing on one PTY (shape 1), plus the web client's chat form as the existing split-UI precedent.

<!-- workflow:view concept-C002 -->

- **State:** `created` · stage S0 · priority `contingent` · horizon `later`
- **Goal:** A player can issue commands mid-game without leaving the world screen, with a visible mode and no mis-routed input.
- **Items:** none yet
- **Relations:**
  - `optional-if` [C001](#c001): The toggle and the split UI switch between raw play and the line mode, so they extend it.
  - `recommended-after` [C006](#c006): The command buffer the toggle opens speaks the shared grammar (#535 D1-045 after #546).
  - `recommended-after` [C003](#c003): The split UI's pinned command row must survive world redraws (L299-L300; #535 D1-046 after #562).
- **Referenced by:** [C003](#c003) `enables`, [C011](#c011) `optional-if`
- **Issues and PRs:**
  - Issue [#535](https://github.com/developing-today/code/issues/535) · S0 · `open` — id(ssh): input and presentation designs; Tracker items D1-045 (in-session toggle) and D1-046 (split UI).

<!-- /workflow:view concept-C002 -->

<a id="c003"></a>

## C003: Alternate screen, incremental rendering and redraw batching

Document: [concepts/C003-terminal-rendering.md](concepts/C003-terminal-rendering.md).

Takes the output side: today's per-event full clear-repaint on the primary screen, moving raw sessions to the alternate screen, a diff renderer that sends only changed cells, and server-side redraw batching, measured against the F11.9 baseline and related to F12.11 delta frames.

<!-- workflow:view concept-C003 -->

- **State:** `created` · stage S0 · priority `recommended` · horizon `next`
- **Goal:** Interactive sessions repaint only what changed, on the alternate screen, at a bounded frame rate, without filling scrollback.
- **Items:** none yet
- **Relations:**
  - `enables` [C002](#c002): In-place, incremental redraw is what lets a world redraw without clobbering a command row (L299-L300).
  - `enables` [C004](#c004): Every tier benefits from batching, and tier 2 depends on the diff renderer (L743-L747, L751).
- **Referenced by:** [C002](#c002) `recommended-after`, [C004](#c004) `recommended-after`
- **Issues and PRs:**
  - Issue [#536](https://github.com/developing-today/code/issues/536) · S0 · `open` — id(transport): transport designs; Tracker item D1-029, F12.11 delta and binary frames, the protocol counterpart of the diff renderer.
  - Issue [#545](https://github.com/developing-today/code/issues/545) · S0 · `open` — id(ssh): use the alternate screen for raw interactive SSH sessions; Alternate screen for raw interactive sessions (D1-042).
  - Issue [#561](https://github.com/developing-today/code/issues/561) · S0 · `open` — id: server-side redraw batching (few-ms frame coalescing) for all presentations; Server-side redraw batching (D1-043).
  - Issue [#562](https://github.com/developing-today/code/issues/562) · S0 · `open` — id(ssh): incremental (diff) ANSI renderer instead of full clear-repaint; Incremental diff ANSI renderer (D1-044).

<!-- /workflow:view concept-C003 -->

<a id="c004"></a>

## C004: Presentation capability tiers and the kitty keyboard protocol

Document: [concepts/C004-presentation-tiers.md](concepts/C004-presentation-tiers.md).

Takes the agreed ghostty-class default presentation with graceful degradation: in-band terminal capability detection, three tiers (kitty keyboard, graphics and synchronized output; cell grid with batching; line mode), and the kitty keyboard protocol on the raw input path.

<!-- workflow:view concept-C004 -->

- **State:** `created` · stage S0 · priority `recommended` · horizon `later`
- **Goal:** Ghostty-class terminals get the full experience and every other terminal degrades predictably, detected in-band over any transport.
- **Items:** none yet
- **Relations:**
  - `recommended-after` [C003](#c003): Tier design builds on the alternate screen, batching and the diff renderer (#535 D1-052 after #545, #561, #562).
- **Referenced by:** [C003](#c003) `enables`, [C001](#c001) `enables`
- **Issues and PRs:**
  - Issue [#535](https://github.com/developing-today/code/issues/535) · S0 · `open` — id(ssh): input and presentation designs; Tracker items D1-052 (presentation tiers) and D1-053 (kitty keyboard on raw input).

<!-- /workflow:view concept-C004 -->

<a id="c005"></a>

## C005: Documents as a session frame pair

Document: [concepts/C005-documents-frame-pair.md](concepts/C005-documents-frame-pair.md).

Takes the world documents path: today an SSH-local read of the server's blob store, and the proposed documents/document ClientFrame/ServerFrame pair in world_session.rs (like records) for web and CLI parity, with its bounds and the per-world store question.

<!-- workflow:view concept-C005 -->

- **State:** `created` · stage S0 · priority `recommended` · horizon `later`
- **Goal:** Web, CLI and SSH clients read a world's documents through one authorized protocol path.
- **Items:** none yet
- **Relations:**
  - `recommended-after` [C001](#c001): The frame pair replaces the SSH-local document path whose gaps #548 fixes first (#535 D1-040 after #548).
- **Issues and PRs:**
  - Issue [#535](https://github.com/developing-today/code/issues/535) · S0 · `open` — id(ssh): input and presentation designs; Tracker item D1-040 (documents as a session frame pair).
  - Issue [#548](https://github.com/developing-today/code/issues/548) · S0 · `open` — id(ssh): fix #211 reconciliation gaps: total records cap, document output through \`send\`, per-world document store; Prerequisite gaps: document output through send and a per-world document store.

<!-- /workflow:view concept-C005 -->

<a id="c006"></a>

## C006: One command grammar and --json output

Document: [concepts/C006-command-grammar-json.md](concepts/C006-command-grammar-json.md).

Takes the design rule that one line grammar mirrors the id world subcommands across SSH command mode, the id world join stdin pump and CLI one-offs, the explicit --command-mode flag, the chat syntax question, and later --json transcript output for agents and CI.

<!-- workflow:view concept-C006 -->

- **State:** `created` · stage S0 · priority `crucial` · horizon `next`
- **Goal:** The same command sentence works as an SSH line, a piped script line and a CLI one-off, with deterministic machine-readable output.
- **Items:** none yet
- **Relations:**
  - `enables` [C011](#c011): The partial-line surface waits until the command grammar is known (L375-L376).
- **Referenced by:** [C002](#c002) `recommended-after`, [C011](#c011) `recommended-after`
- **Issues and PRs:**
  - Issue [#546](https://github.com/developing-today/code/issues/546) · S0 · `open` — id: one line grammar for SSH command mode, the \`id world join\` pump and CLI verbs (\`--command-mode\`); One line grammar and --command-mode (D1-041).
  - Issue [#563](https://github.com/developing-today/code/issues/563) · S0 · `open` — id: \`--json\` transcript output in world line mode; --json transcript output in line mode, after #546.

<!-- /workflow:view concept-C006 -->

<a id="c007"></a>

## C007: SSH as a frame transport and SSH-over-iroh

Document: [concepts/C007-ssh-transport.md](concepts/C007-ssh-transport.md).

Takes the transport layer: the envelope/session/ABI layer model, SSH versus iroh QUIC, SSH as a transport for the JSON frame protocol over an exec channel (id world join --via ssh), two SSH channels on one connection (shape 2), SSH-over-iroh and iroh-over-SSH tunnels, and ssh3 status.

<!-- workflow:view concept-C007 -->

- **State:** `created` · stage S0 · priority `contingent` · horizon `later`
- **Goal:** Clients can reach worlds over SSH where QUIC cannot go, and third-party SSH clients can reach NAT'd hosts, without SSH becoming the default.
- **Items:** none yet
- **Relations:**
  - `optional-if` [C001](#c001): SSH as a frame transport and a second SSH channel attach to the same SSH server, at exec\_request and subsystem\_request (L190-L192, L467-L468).
  - `enables` [C010](#c010): SSH-over-iroh and SSH as a transport widen which clients reach which servers (L714-L719), which the multi-server topology uses (L768-L771).
- **Issues and PRs:**
  - Issue [#536](https://github.com/developing-today/code/issues/536) · S0 · `open` — id(transport): transport designs; Tracker items D1-050 (SSH as a frame transport) and D1-051 (SSH-over-iroh).
  - Issue [#564](https://github.com/developing-today/code/issues/564) · S0 · `open` — id: write the Phase 13 plan: client shells and client capability providers; The Phase 13 plan holds D1-050 and D1-051.

<!-- /workflow:view concept-C007 -->

<a id="c008"></a>

## C008: Native client shells

Document: [concepts/C008-native-client-shells.md](concepts/C008-native-client-shells.md).

Takes native client shells (Bevy, Qt, .NET, Unity, Electron, Windows desktop) implementing the same screen/input contract for what a terminal cannot do, made swappable by the portable surface contract and the F12.7 ABI conformance suite.

<!-- workflow:view concept-C008 -->

- **State:** `created` · stage S0 · priority `recommended` · horizon `later`
- **Goal:** A native shell can replace the terminal or browser by implementing the screen/input contract and passing conformance.
- **Items:** none yet
- **Relations:** none
- **Issues and PRs:**
  - Issue [#538](https://github.com/developing-today/code/issues/538) · S0 · `open` — id: native client shells and client capabilities; Tracker item D1-054 (native client shells).
  - Issue [#564](https://github.com/developing-today/code/issues/564) · S0 · `open` — id: write the Phase 13 plan: client shells and client capability providers; The Phase 13 plan holds D1-054.

<!-- /workflow:view concept-C008 -->

<a id="c009"></a>

## C009: Client-side capability providers

Document: [concepts/C009-client-capability-providers.md](concepts/C009-client-capability-providers.md).

Takes client-side capabilities: apps that need the client's file system, GPU and network get them through named, granted, ceilinged providers on the client, extending scopes, roles, ceilings and the F12.4 provider registry without a new authority path.

<!-- workflow:view concept-C009 -->

- **State:** `created` · stage S0 · priority `recommended` · horizon `later`
- **Goal:** Apps can use client-local files, GPU and network only through named, granted, ceilinged client providers.
- **Items:** none yet
- **Relations:** none
- **Issues and PRs:**
  - Issue [#538](https://github.com/developing-today/code/issues/538) · S0 · `open` — id: native client shells and client capabilities; Tracker item D1-055 (client-side capability providers).
  - Issue [#564](https://github.com/developing-today/code/issues/564) · S0 · `open` — id: write the Phase 13 plan: client shells and client capability providers; The Phase 13 plan holds D1-055.

<!-- /workflow:view concept-C009 -->

<a id="c010"></a>

## C010: Multi-server and multi-client topology

Document: [concepts/C010-multi-server-multi-client.md](concepts/C010-multi-server-multi-client.md).

Takes the topology: one client connected to several worlds, servers and other clients at once, id as a client that is often also a server, client-only mode always supported, and the existing iroh client and server-to-server transports it builds on.

<!-- workflow:view concept-C010 -->

- **State:** `created` · stage S0 · priority `recommended` · horizon `later`
- **Goal:** One id client can join many worlds and servers at once and act as a server too, while client-only mode stays supported.
- **Items:** none yet
- **Relations:** none
- **Referenced by:** [C007](#c007) `enables`
- **Issues and PRs:**
  - Issue [#536](https://github.com/developing-today/code/issues/536) · S0 · `open` — id(transport): transport designs; Tracker item D1-056 (multi-server, multi-client topology).
  - Issue [#564](https://github.com/developing-today/code/issues/564) · S0 · `open` — id: write the Phase 13 plan: client shells and client capability providers; The Phase 13 plan holds D1-056.

<!-- /workflow:view concept-C010 -->

<a id="c011"></a>

## C011: Roc partial-line surface

Document: [concepts/C011-roc-partial-line.md](concepts/C011-roc-partial-line.md).

Takes the guest-facing extension: a bounded, never-journaled notification telling a Roc program what the viewer is typing, so apps can render live autocomplete or a validating form, and how guest input and host events reach programs today.

<!-- workflow:view concept-C011 -->

- **State:** `created` · stage S0 · priority `contingent` · horizon `later`
- **Goal:** A Roc app can react to what the viewer is typing without the partial line ever reaching the journal.
- **Items:** none yet
- **Relations:**
  - `recommended-after` [C006](#c006): Deferred until the line mode exists and the grammar is known (L375-L376).
  - `optional-if` [C002](#c002): The partial line lives in the split UI's command region (L345-L351; #535 D1-048 after D1-046).
- **Referenced by:** [C006](#c006) `enables`
- **Issues and PRs:**
  - Issue [#535](https://github.com/developing-today/code/issues/535) · S0 · `open` — id(ssh): input and presentation designs; Tracker item D1-048 (Roc partial-line surface).

<!-- /workflow:view concept-C011 -->

<a id="c012"></a>

## C012: G: global directives and report context

Document: [concepts/C012-global-context.md](concepts/C012-global-context.md).

Stand-in for the global bucket G, which the tool on main lacks. Holds the report's framing, the constraints every concept follows, and the material outside the client concepts: what landed, app state, CI, Phase 11 and 12 plans, open issues, verification and corrections. Not researched at S1.

<!-- workflow:view concept-C012 -->

- **State:** `created` · stage S0
- **Goal:** Keep the report's framing, constraints and out-of-scope work visible to every concept without researching it here.
- **Items:** none yet
- **Relations:** none
- **Issues and PRs:** none yet

<!-- /workflow:view concept-C012 -->

## Issues and PRs

<!-- workflow:view issues-and-prs -->

- [C001](#c001) SSH command (line) mode and the #211 gaps
  - Issue [#548](https://github.com/developing-today/code/issues/548) · S0 · `open` — id(ssh): fix #211 reconciliation gaps: total records cap, document output through \`send\`, per-world document store; The #211 gaps: total records cap, document output through send, per-world store (D1-039).
  - Issue [#549](https://github.com/developing-today/code/issues/549) · S0 · `open` — id(ssh): SSH input queue silently drops keystrokes when full; Full input queue drops keystrokes silently (D1-058).
- [C002](#c002) In-session command toggle and split UI
  - Issue [#535](https://github.com/developing-today/code/issues/535) · S0 · `open` — id(ssh): input and presentation designs; Tracker items D1-045 (in-session toggle) and D1-046 (split UI).
- [C003](#c003) Alternate screen, incremental rendering and redraw batching
  - Issue [#536](https://github.com/developing-today/code/issues/536) · S0 · `open` — id(transport): transport designs; Tracker item D1-029, F12.11 delta and binary frames, the protocol counterpart of the diff renderer.
  - Issue [#545](https://github.com/developing-today/code/issues/545) · S0 · `open` — id(ssh): use the alternate screen for raw interactive SSH sessions; Alternate screen for raw interactive sessions (D1-042).
  - Issue [#561](https://github.com/developing-today/code/issues/561) · S0 · `open` — id: server-side redraw batching (few-ms frame coalescing) for all presentations; Server-side redraw batching (D1-043).
  - Issue [#562](https://github.com/developing-today/code/issues/562) · S0 · `open` — id(ssh): incremental (diff) ANSI renderer instead of full clear-repaint; Incremental diff ANSI renderer (D1-044).
- [C004](#c004) Presentation capability tiers and the kitty keyboard protocol
  - Issue [#535](https://github.com/developing-today/code/issues/535) · S0 · `open` — id(ssh): input and presentation designs; Tracker items D1-052 (presentation tiers) and D1-053 (kitty keyboard on raw input).
- [C005](#c005) Documents as a session frame pair
  - Issue [#535](https://github.com/developing-today/code/issues/535) · S0 · `open` — id(ssh): input and presentation designs; Tracker item D1-040 (documents as a session frame pair).
  - Issue [#548](https://github.com/developing-today/code/issues/548) · S0 · `open` — id(ssh): fix #211 reconciliation gaps: total records cap, document output through \`send\`, per-world document store; Prerequisite gaps: document output through send and a per-world document store.
- [C006](#c006) One command grammar and --json output
  - Issue [#546](https://github.com/developing-today/code/issues/546) · S0 · `open` — id: one line grammar for SSH command mode, the \`id world join\` pump and CLI verbs (\`--command-mode\`); One line grammar and --command-mode (D1-041).
  - Issue [#563](https://github.com/developing-today/code/issues/563) · S0 · `open` — id: \`--json\` transcript output in world line mode; --json transcript output in line mode, after #546.
- [C007](#c007) SSH as a frame transport and SSH-over-iroh
  - Issue [#536](https://github.com/developing-today/code/issues/536) · S0 · `open` — id(transport): transport designs; Tracker items D1-050 (SSH as a frame transport) and D1-051 (SSH-over-iroh).
  - Issue [#564](https://github.com/developing-today/code/issues/564) · S0 · `open` — id: write the Phase 13 plan: client shells and client capability providers; The Phase 13 plan holds D1-050 and D1-051.
- [C008](#c008) Native client shells
  - Issue [#538](https://github.com/developing-today/code/issues/538) · S0 · `open` — id: native client shells and client capabilities; Tracker item D1-054 (native client shells).
  - Issue [#564](https://github.com/developing-today/code/issues/564) · S0 · `open` — id: write the Phase 13 plan: client shells and client capability providers; The Phase 13 plan holds D1-054.
- [C009](#c009) Client-side capability providers
  - Issue [#538](https://github.com/developing-today/code/issues/538) · S0 · `open` — id: native client shells and client capabilities; Tracker item D1-055 (client-side capability providers).
  - Issue [#564](https://github.com/developing-today/code/issues/564) · S0 · `open` — id: write the Phase 13 plan: client shells and client capability providers; The Phase 13 plan holds D1-055.
- [C010](#c010) Multi-server and multi-client topology
  - Issue [#536](https://github.com/developing-today/code/issues/536) · S0 · `open` — id(transport): transport designs; Tracker item D1-056 (multi-server, multi-client topology).
  - Issue [#564](https://github.com/developing-today/code/issues/564) · S0 · `open` — id: write the Phase 13 plan: client shells and client capability providers; The Phase 13 plan holds D1-056.
- [C011](#c011) Roc partial-line surface
  - Issue [#535](https://github.com/developing-today/code/issues/535) · S0 · `open` — id(ssh): input and presentation designs; Tracker item D1-048 (Roc partial-line surface).

<!-- /workflow:view issues-and-prs -->
