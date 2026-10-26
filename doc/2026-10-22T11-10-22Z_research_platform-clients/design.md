# Initial review and research order

[Main reference](main.md) | [Transcript](transcript.txt)

Hard prerequisites use `requires`; other edges express sequencing cost, conflicts, shared files or optional enabling work.

<!-- workflow:view waves -->

Wave 1 can start now: everything it requires is done. Later waves assume the earlier ones finish. Concepts that share files never share a wave. Leases and open branches are live data, so run `ready` before dispatching.

- **Done:** none
- **Running:** none
- **Wave 1:** [C001](main.md#c001), [C002](main.md#c002), [C003](main.md#c003), [C004](main.md#c004), [C005](main.md#c005), [C006](main.md#c006), [C007](main.md#c007), [C008](main.md#c008), [C009](main.md#c009), [C010](main.md#c010), [C011](main.md#c011), [C012](main.md#c012)
  - [C002](main.md#c002) is better after [C006](main.md#c006): The toggle prompt's command set diverges from the grammar and has to be redone.
  - [C002](main.md#c002) is better after [C003](main.md#c003): A pinned row designed against full clear-repaint is redone for the diff renderer.
  - [C004](main.md#c004) is better after [C003](main.md#c003): Tier-2 behaviour is specified against a renderer that then changes.
  - [C005](main.md#c005) is better after [C001](main.md#c001): The frame pair is designed before document output goes through send and the per-world store is decided.
  - [C011](main.md#c011) is better after [C006](main.md#c006): The partial-line event is shaped around commands that the grammar later renames.

<!-- /workflow:view waves -->

## Dependencies

- C002 optional-if C001: The toggle and the split UI switch between raw play and the line mode, so they extend it.
- C002 recommended-after C006: The command buffer the toggle opens speaks the shared grammar (#535 D1-045 after #546).
- C002 recommended-after C003: The split UI's pinned command row must survive world redraws (L299-L300; #535 D1-046 after #562).
- C003 enables C002: In-place, incremental redraw is what lets a world redraw without clobbering a command row (L299-L300).
- C003 enables C004: Every tier benefits from batching, and tier 2 depends on the diff renderer (L743-L747, L751).
- C004 recommended-after C003: Tier design builds on the alternate screen, batching and the diff renderer (#535 D1-052 after #545, #561, #562).
- C001 enables C004: The line mode is the tier-3 degraded presentation (L748-L749).
- C005 recommended-after C001: The frame pair replaces the SSH-local document path whose gaps #548 fixes first (#535 D1-040 after #548).
- C006 enables C011: The partial-line surface waits until the command grammar is known (L375-L376).
- C011 recommended-after C006: Deferred until the line mode exists and the grammar is known (L375-L376).
- C011 optional-if C002: The partial line lives in the split UI's command region (L345-L351; #535 D1-048 after D1-046).
- C007 optional-if C001: SSH as a frame transport and a second SSH channel attach to the same SSH server, at exec_request and subsystem_request (L190-L192, L467-L468).
- C007 enables C010: SSH-over-iroh and SSH as a transport widen which clients reach which servers (L714-L719), which the multi-server topology uses (L768-L771).

## Review status

<!-- workflow:view run-status -->

S0 is `validated` when `render` runs, because render only runs on a package that passes `check`. Other stages show the `tasks` entries that record `concept`, `stage` and `status`. `pushed`, `pr-open` and `merged` come from git and GitHub through `status --live`, so they never appear here.

- **C001** [SSH command (line) mode and the #211 gaps](main.md#c001): S0 `validated` ([#548](https://github.com/developing-today/code/issues/548), [#549](https://github.com/developing-today/code/issues/549)) · S1–S8 `not-started`
- **C002** [In-session command toggle and split UI](main.md#c002): S0 `validated` ([#535](https://github.com/developing-today/code/issues/535)) · S1–S8 `not-started`
- **C003** [Alternate screen, incremental rendering and redraw batching](main.md#c003): S0 `validated` ([#536](https://github.com/developing-today/code/issues/536), [#545](https://github.com/developing-today/code/issues/545), [#561](https://github.com/developing-today/code/issues/561), [#562](https://github.com/developing-today/code/issues/562)) · S1–S8 `not-started`
- **C004** [Presentation capability tiers and the kitty keyboard protocol](main.md#c004): S0 `validated` ([#535](https://github.com/developing-today/code/issues/535)) · S1–S8 `not-started`
- **C005** [Documents as a session frame pair](main.md#c005): S0 `validated` ([#535](https://github.com/developing-today/code/issues/535), [#548](https://github.com/developing-today/code/issues/548)) · S1–S8 `not-started`
- **C006** [One command grammar and --json output](main.md#c006): S0 `validated` ([#546](https://github.com/developing-today/code/issues/546), [#563](https://github.com/developing-today/code/issues/563)) · S1–S8 `not-started`
- **C007** [SSH as a frame transport and SSH-over-iroh](main.md#c007): S0 `validated` ([#536](https://github.com/developing-today/code/issues/536), [#564](https://github.com/developing-today/code/issues/564)) · S1–S8 `not-started`
- **C008** [Native client shells](main.md#c008): S0 `validated` ([#538](https://github.com/developing-today/code/issues/538), [#564](https://github.com/developing-today/code/issues/564)) · S1–S8 `not-started`
- **C009** [Client-side capability providers](main.md#c009): S0 `validated` ([#538](https://github.com/developing-today/code/issues/538), [#564](https://github.com/developing-today/code/issues/564)) · S1–S8 `not-started`
- **C010** [Multi-server and multi-client topology](main.md#c010): S0 `validated` ([#536](https://github.com/developing-today/code/issues/536), [#564](https://github.com/developing-today/code/issues/564)) · S1–S8 `not-started`
- **C011** [Roc partial-line surface](main.md#c011): S0 `validated` ([#535](https://github.com/developing-today/code/issues/535)) · S1–S8 `not-started`
- **C012** [G: global directives and report context](main.md#c012): S0 `validated` · S1–S8 `not-started`

<!-- /workflow:view run-status -->
