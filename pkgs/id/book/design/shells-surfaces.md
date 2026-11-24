# Design: shells and client surfaces

This chapter is design. It describes the shell and client-surface system as designed in the programme's design archive: 43 records homed here in the survey of the archive for this book (tracking issue #13332; programme tracker #13240). A shell is a client program that speaks the shell contract to a world host, renders the host's frames, and sends the client's frames back. The design rule that runs through all 43 records is that shells differ in presentation and input, and do not differ in authority: every shell speaks the same contract, and no shell gets a private frame vocabulary. Where a behaviour in this chapter has not shipped, the text says so; the final section states what exists today.

Related chapters: [Design: worlds and the guest runtime](worlds-runtime.md), [Design: capabilities and providers](capabilities-providers.md), [Shells: terminal, web, native and hardware](../concepts/shells.md).

## The shell contract is the centre of the design

The contract of record is `2026-10-25T05-54-40Z_reference_shell_contract` (2026-10-25), the prose form of the machine-readable contract in `pkgs/id/contract/shell-contract.v1.json`: contract name `id-shell`, version 1, revision 15. The design fixes these values:

- Protocol: ALPN `/id-world/1`.
- Transports: three — an iroh QUIC stream, WebSocket at `/ws/world`, and SSH exec channel `id-world/1`.
- Maximum frame size: 1 MiB.
- Session phases: four — opening, query, install, joined.
- Frame classes: six — event, ephemeral, query, control, admin, delegation.
- Client and server vocabularies covering join, records, documents, caps, invite, directory, list_worlds, create_world, isolation, cross-world and file-scope admin frames, and compile/install frames.

Conformance is a suite, not an opinion. `pkgs/id/contract/shell-suite.v1.json` names four shells — terminal, browser, native, hardware — and states, for each shell, the frame sets it presents and sends, its declared providers, and its workflow. Only the native shell declares a provider in the suite: `client.file.read`. Two golden session transcripts pin the expected exchange: `sessions/terminal.jsonl` with 5 frames and `sessions/native.jsonl` with 7 frames, each running join, snapshot, resize, view, with the native transcript also carrying scene and audio surface payloads. A shell that fails the suite does not join a world; the agents design in [Design: agents in worlds and the MCP bridge](agents-mcp.md) adopts the same rule for agent clients.

## The four shell lineages

### Terminal

The terminal lineage starts before the contract existed. `2026-04-01T00-00-00Z_feature_image_upload` (2026-04-01) covers paste, drag-drop and toolbar insert for images in the early editor surface. The October 2026 records build the world-facing terminal: `2026-10-15T16-00-00Z_feature_world_viewport_and_paste` (per-session viewport and paste policy), `2026-10-15T16-30-00Z_feature_terminal_buffer_capture` (a terminal buffer capture interface), and `2026-10-16T13-00-00Z_feature_world_apps_structured_migration` with `2026-10-16T14-00-00Z_feature_screen_render_alignment`, which move world apps onto the structured Screen presentation instead of raw text.

Input has its own records. `2026-10-24T20-18-37Z_decision_ssh_input_modes` (2026-10-24) fixes SSH input: raw mode is the default, `cmd:` selects line mode, and a username grammar names the target. `2026-11-02T_reference_kitty_disambiguate_v1.md` is the reference for the kitty keyboard disambiguation profile, and the Phase 13 record for the kitty protocol itself is homed with the identity records and cross-linked from here. Output safety is a terminal problem first: `2026-11-01_design_id3455.md` (2026-11-01) states the contract that a value cannot act on the terminal — one shared Unicode format-character (category Cf) set for all plain output in `pkgs/id`, with row renderers escaping, sanitizers stripping only where their output contract calls for stripping, and validators refusing only where their contract calls for refusal. `2026-11-08_design_id5785.md` extends the same line past plain text: markup contexts, safe-plain as a user choice, ingest without evaluation, and a stated disposition for ANSI and OSC sequences. `2026-11-08_design_id5873.md` designs terminal participation in object discussions, so the terminal is a discussion surface and not only a world surface.

### Browser

The browser shell renders the same Screen documents as DOM. `2026-10-16T19-00-00Z_feature_guest_reflow` gives guests a `layout` entry point so a program renders against the actual viewport instead of a fixed grid, and `2026-10-16T19-30-00Z_feature_screen_widgets` adds screen widgets and the `RocText` library. The boards design, `2026-11-01_design_id2054.md`, specifies signed boards with portable themes and playable posts as a browser-first surface. The web rendering decision for the semantic pipeline is `2026-11-09_decision_id5851_assertion_tailwind_mapping.md`: a style assertion maps to Tailwind classes on the web target. The shipped web UI itself — the file browser and collaborative editor — is documented in [The web UI: pages and HTTP API](../reference/web-ui.md); this chapter covers only its place in the shell design: it is one shell among four, bound by the same contract when it joins a world.

### Native

The native reference shell is F13.6, designed in `2026-11-01_design_id788.md` (2026-11-01): a desktop client that connects to a world host over the session protocol, renders the host's view frames, and emits only the client frames the contract already names — `input`, `paste`, `resize`, `chat`. It adds no frame kinds and needs no host change. `2026-11-08_design_id5781.md` designs the deeper Bevy incorporation: ID worlds as Bevy applications, with the journal as the only authority and the Bevy world as a materialized projection of journal positions; a Bevy application never writes world state directly, and every mutation becomes an intent submitted to the world's append gate. The native-rendering decisions sit beside it: `2026-11-08_decision_id5852_native_rendering_os_standard_controls.md` (a semantic assertion renders as an OS-standard control on native targets) and `2026-11-08_decision_id5853_custom_gui_rendering_overrides.md` (custom GUI rendering is a first-class override, not a fork of the document).

### Hardware and mobile

`2026-11-10T20-40-00Z_design_more_shells_v1` (2026-11-10) is the first-iteration design for the remaining shells: mobile, native and hardware in one ladder. Its hardware slice reduces the contract to the subset that fits an ESP32-class device: presence, journal-head state in compressed form, and confirm/act through form results. Rich rendering, free text and world hosting are excluded by name. `2026-11-08T22-06-00Z_design_id_android_client` (2026-11-08) designs a native Android client. `2026-11-23T03-17-14Z_design_more_shells_v2` (2026-11-23) is an outline only — it states that no implementation is landed or claimed by the record — and scopes the next rung: native and TUI completion, with a hardware-shell rung (#6298) and a third-party-shells rung (#6299) named as siblings.

## Presentation: one document, many renderers

The presentation design separates what a program means from how a shell draws it.

- `2026-10-31T17-47-20Z_design_world_presentation_and_fuel` (2026-10-31) records the current model of world presentation, rendering and fuel together: presentation work is guest work and is charged against the guest's fuel budget.
- `2026-11-08_decision_id5849_semantic_markup_vocabulary_v1.md` (2026-11-08, from #5679) fixes the semantic markup vocabulary: the element set and the authoring syntax that the rendering decisions consume.
- `2026-11-08_decision_id5854_per_target_composition.md` decides per-target composition: themes, overrides, assets and conditional blocks are selected per target from one source document.
- `2026-11-08_decision_id5855_rendering_pipeline_conformance.md` decides the pipeline split — compile-time and runtime stages — with per-target expected-output tests and author tooling and linting.
- `2026-11-17_design_id6010_semantic_cell_tag_model.md` (2026-11-17) states the presentation model in one line: one tagged state, many renderers — a semantic cell/tag model in which each renderer consumes the same tagged state.
- `2026-11-10_design_id5858.md` decides the portable theme format and its precedence rules.
- `2026-11-08_design_id6012_visual_feature_cost_ranking.md` ranks visual features by cost, so a shell with a small budget can drop features in a designed order instead of failing at random.
- `2026-11-08_decision_id5910_kdl_presentation_surfaces.md` and `2026-11-14_decision_id5907_triple_decomposition.md` cover the KDL presentation surfaces — export, hand upload, agent statements — and the rules for decomposing KDL statements into triples.

The fallback rule is uniform across the design and the shipped shells: a shell without a renderer for a surface keeps that surface's text projection. Scene, audio and widget surfaces degrade to text; they never block the session.

## Query and client parity

Three records make parity a design requirement rather than a hope. `2026-11-08T06-45-00Z_design_id_query_local_remote_v1` designs querying local and remote ID servers under one v1 model. `2026-11-08_design_id5820.md` (2026-11-08) decides that the scripting/API front door, the agent front door and the CLI front door expose one query model, and that parity between the front doors is proven, not asserted; a front-door convenience that would fork the parent query decisions (#5675) is rejected in the record rather than worked around. `2026-11-10_decision_id5818_remote_filtered_query.md` decides server-side filtered remote query execution, and `2026-11-17T15-40-00Z_design_id_federated_query_fanout` (#9144, 2026-11-17) designs fan-out across federated peers. `2026-11-09T01-10-00Z_design_id_search_ranking` is the decision record for personalized search ranking for notes, homed here by the survey and cross-linked from [Design: the knowledge layer](knowledge-layer.md).

## Sessions, topology and program-facing surfaces

`2026-11-01T17-54-35Z_design_id_concurrent_world_sessions` (F13.8) designs the client coordinator for concurrent world sessions: one client holding several world sessions at once. Its sibling, `2026-11-01_design_id790.md`, designs the multi-server and multi-client topology those sessions run in. `2026-11-08T08-10-00Z_design_id5782_graphical_client_tooling.md` is the v1 design for the graphical client and tooling surface. Three records cover how programs reach shells: `2026-11-14T13-50-00Z_design_id_roc_library_imports` (Roc library imports for worlds — platform-requirement representation, subset checking, provenance, import errors), `2026-11-16T13-00-00Z_design_id5964_dynamic_user_functions` (dynamic user functions — interface contract, compile pipeline, v1 placement), and `2026-11-08T10-26-00Z_design_id_per_user_function_mapping` (per-user function mapping — bot battles, strategy hot-switching, human and AI partner composition). `2026-11-08T10-26-00Z_design_id5996_hot_swap_decision` decides the v1 hot-swap mechanism among its alternatives.

## What exists today

Grounding, from the code and contract surveys for this book (checkout `1ccb4a19457`, 2026-11-23):

- Shipped: the machine-readable shell contract in `pkgs/id/contract/` at revision 15, with the four-shell conformance suite and the two golden transcripts. The contract is the design's centre and it exists as data, not only as prose.
- Shipped: the terminal and browser paths of the main `id` program, including the web UI (file browser, ProseMirror collaborative editor) documented in [The web UI: pages and HTTP API](../reference/web-ui.md).
- Shipped in part: the native shell crate `id-native-shell`. Steps 1 and 2 of design #788 are in the code — the isolated package, identity and connection bootstrap, contract frame builders, view fallback, the transcript fixture checked against the contract, the window and render plugin set, and a live session loop. It is a workspace member but not a default member, so a plain build of `id` never acquires its Bevy dependencies. The full design in `2026-11-08_design_id5781.md` — worlds hosted inside Bevy applications — is design only; the shipped shell is a presenter client.
- Shipped as a spike: the hardware shell crate `id-hw-shell`, complete at its stated subset (presence, journal head, confirm/act; only `join` and `form_result` client frames; a form requiring free text is refused). It is transport-free frame-building and folding code, not a running device firmware.
- Still design: the Android client (`2026-11-08T22-06-00Z_design_id_android_client`), the more-shells v2 completion rung (`2026-11-23T03-17-14Z_design_more_shells_v2`, outline only by its own statement), third-party shells (#6299), the semantic rendering pipeline as a complete system (`id-render` exists as a crate with its own test suites; the five November 2026 decisions describe the target pipeline, and this chapter does not claim every decision is implemented — that claim is unverified from the records read for this chapter), and federated query fan-out (#9144).
