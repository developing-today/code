# Design: worlds and the guest runtime

This chapter covers 20 design records: the worlds design, the Roc guest runtime, and the contract between a world host and the shells that connect to it. It describes the system as designed in the programme's design archive. The final section states what runs today.

The worlds programme is tracked as #5955 (Roc libraries, dynamic user functions, presentation assets), with children that include #5973 (world governance), #6006 (threat-list feeds) and #5985/#5986 (licensing and hosting research). Worlds also sit inside the core epic #2056 and inside Phase 12 (#534) and Phase 13 of `pkgs/id`.

## Roc-defined multiplayer worlds

The founding record is `2026-10-07T00-20-00Z_architecture_roc_worlds` (2026-10-07), "Roc-Defined Multiplayer Worlds". The design decision it records is the one the whole programme follows: a world is a multiplayer program written in Roc, compiled to a guest module, and run by a host that owns storage, networking and authority. The guest computes; the host admits, persists and enforces. Guest programs reach the host only through the capability protocol covered in [Design: capabilities and providers](capabilities-providers.md).

The platform world is the reference world. Two records track it: `2026-10-18T00-00-00Z_reference_platform_state` (2026-10-18), "Platform world: current state, findings, and open work", and `2026-10-18T18-40-00Z_report_platform_world_state` (2026-10-18), the state report of the same date. Together they are the design archive's own status record for the first world, distinct from the design records around it.

`2026-11-08T09-10-00Z_research_worlds_conversation_archaeology` (2026-11-08) is the programme's archaeology record: it ties earlier discussions of Second Life and programmable worlds into the #5955 programme, so the design's lineage is recorded rather than reconstructed later.

## Isolation, cross-world access and execution profiles

Three records from 2026-10-10 define how worlds are separated and how they reach each other:

- `2026-10-10T03-56-25Z_feature_world_isolation` (2026-10-10) designs world isolation modes: the set of isolation postures a host can run a world under.
- `2026-10-10T04-59-57Z_feature_cross_world_access` (2026-10-10) designs cross-world access in terms of writes, force, and file scope: a cross-world write is a distinct, named operation with a file scope, not a side effect of holding a capability in the other world.
- `2026-10-14T19-39-30Z_architecture_portable_world_surfaces` (2026-10-14) designs portable world surfaces and controlled execution profiles, so the same world can present on different surfaces under a stated execution profile.

`2026-10-30T19-36-00Z_investigation_world_vm_wasm_portability` (2026-10-30) audits the portability question directly: what of the worlds runtime runs inside a Web Worker. The audit is the design input for the hosted edition's world support, covered in [Design: hosted and serverless editions](hosted-editions.md).

## Module provenance and installs

A host runs code it did not write, so the design gives modules a provenance chain:

- `2026-10-14T20-00-05Z_feature_source_only_world_installs` (2026-10-14) designs source-only world module installation: the install unit is source, and the host compiles it.
- `2026-10-16T09-00-00Z_feature_world_module_provenance` (2026-10-16, 09:00) designs observed world module provenance — the provenance the host observes at runtime.
- `2026-10-16T09-30-00Z_feature_world_module_provenance` (2026-10-16, 09:30) designs world module build provenance — the provenance recorded at build time. The two records share a timestamp prefix and a name; the survey counts them as distinct records, observed provenance and build provenance.
- `2026-11-02T00-31-45Z_design_module_trust` (2026-11-02) designs module trust as an admin trust list plus an install-time signature check. The survey homes this record in the identity chapter; it is cross-linked here because install-time trust is a runtime gate. See [Design: identity, directory, groups and delegation](identity-directory-groups.md).

Licensing constrains hosting: `2026-11-05T15-08-16Z_research_id5986_agpl_gpl_hosting` (2026-11-05) researches AGPL/GPL hosting — inspectable worlds, uploads, and self-hosting (#5986, under #5955). The design position the research supports is that world source stays inspectable when a world is hosted for others.

## The guest runtime: Roc support libraries and the authoring loop

The runtime design includes the libraries a guest author writes against:

- `2026-10-15T09-27-28Z_feature_roc_json` (2026-10-15) designs pure-Roc JSON for portable world programs, so guests do not depend on a host JSON service.
- `2026-10-16T16-00-00Z_feature_roc_cap` (2026-10-16), RocCap, designs shared request bookkeeping for the capability protocol on the guest side. The host side of that protocol is in [Design: capabilities and providers](capabilities-providers.md).
- `2026-11-01T16-09-48Z_feature_world_json_transcript` (2026-11-01) designs the `id world join --command-mode --json` transcript, a machine-readable record of a world session.

The authoring loop has three records that describe one workflow at three dates: `2026-10-16T18-30-00Z_feature_world_authoring_cli` (2026-10-16) designs the authoring CLI (`id world new` and `id world stats`); `2026-11-01_design_id558.md` (2026-11-01) designs the `id world dev` authoring loop; `2026-11-16_id_world_dev_authoring.md` (2026-11-16) documents authoring Roc worlds with `id world dev` as a working practice. The progression in the archive is CLI surface, then loop design, then authoring guide.

## Scheduled events and the host view

Worlds are long-lived, so the design gives them time:

- `2026-10-17T10-00-00Z_feature_scheduled_events` (2026-10-17) designs scheduled events in the minimal form: a world can ask for one fire at a set time.
- `2026-11-03T01-31-11Z_design_recurring_cross_world_schedules` (2026-11-03) extends the design to recurring schedules and cross-world schedules.

`2026-11-02T00-53-51Z_feature_world_host_view` (2026-11-02) designs an authenticated, read-only world host view: the operator of a host can inspect a world without joining it as a participant and without write authority.

## The shell contract's evolution

Worlds are useless without shells, and the design separates the two with a contract. The prose record is `2026-10-25T05-54-40Z_reference_shell_contract` (2026-10-25), "The id shell contract, version 1", which the survey homes in the shells chapter. The normative form lives in `pkgs/id/contract/`: `shell-contract.v1.json` is contract `id-shell`, version 1, revision 15. The contract as recorded there:

- ALPN protocol `/id-world/1`, with three transports: an iroh QUIC stream, a WebSocket at `/ws/world`, and SSH exec `id-world/1`.
- Maximum frame size: 1 MiB.
- Session phases: opening, query, install, joined.
- Six frame classes: event, ephemeral, query, control, admin, delegation.
- A client/server frame vocabulary that includes join, records, documents, caps, invite, directory, `list_worlds`, `create_world`, isolation, cross-world and file-scope admin frames, and compile/install frames.

The conformance side is `shell-suite.v1.json`, which names four shells — terminal, browser, native, hardware — with each shell's presented and sent frame sets. Only the native shell declares a provider in the suite: `client.file.read`. Two golden session transcripts, `sessions/terminal.jsonl` (5 frames) and `sessions/native.jsonl` (7 frames), record join, snapshot, resize and view sequences, including scene and audio surface payloads, for the suite to check shells against.

The design chapters for the shells themselves, and for the agents that join worlds through the MCP bridge, are [Design: shells and client surfaces](shells-surfaces.md) and [Design: agents in worlds and the MCP bridge](agents-mcp.md).

## What exists today

Grounded in the code and contract surveys, not the design records:

- Worlds run today. The default build of the `id` binary includes the world runtime (`default = [sandbox, world]` in the CLI survey's build table), with world join, administration, journal, dev, measure, replay and verify commands in the shipped CLI surface.
- The shell contract exists as a machine-readable file at revision 15, with the four-shell conformance suite and two golden transcripts in `pkgs/id/contract/`. That is a shipped contract artifact, not only a design record.
- The hosted runtime proof is a proof of concept, not a production path: `pkgs/id/worker-poc` runs one example world unmodified in a Cloudflare Worker isolate and proves journal equality against the native wasmtime host (acceptance boxes checked 2026-11-01, per the web survey). Nothing in `worker-poc` is a production path.
- Still design only, with no shipped counterpart verified in the surveys: recurring cross-world schedules as a general facility, and the authenticated read-only host view as a distinct surface. Their shipped status is unverified in the survey material beyond the records cited above.
