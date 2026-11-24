# Census A — doc/ dated-records census (laptop checkout ~/git/code/doc, read 2026-11-23)

## Counts

- Total entries in `doc/`: 619 (`ls doc | wc -l`).
- Dated folders by name prefix (`ls doc | grep -o '^2026-[0-9][0-9]' | sort | uniq -c`):
  - 2026-03: 6
  - 2026-04: 2
  - 2026-10: 103
  - 2026-11: 452
  - (Remainder: undated notes, `living/`, `archive/`, diagrams, PDFs, symlinks — see doc/README.md "Legacy places".)

## The pre-October records (complete list)

- 2026-03-20T05-30-00Z_feature_collaborative_cursor_enhancements
- 2026-03-21T05-47-48Z_feature_content_modes
- 2026-03-22T00-00-00Z_feature_peer-discovery
- 2026-03-23T00:00:00Z_feature_file_list_ui
- 2026-03-23T12-00-00Z_feature_metadata_tags
- 2026-03-29T00-00-00Z_reference_testing_architecture
- 2026-04-01T00-00-00Z_feature_image_upload
- 2026-04-01T00-00-00Z_feature_multi_instance

## October phase/feature records relied on for W41–W42

- 2026-10-07T00-20-00Z_architecture_roc_worlds
- 2026-10-09T09-59-07Z_design_attenuated_capabilities
- 2026-10-09T12-00-00Z_feature_directory_accounts_groups
- 2026-10-10T02-39-03Z_design_directory_sync; 2026-10-10T03-56-25Z_feature_world_isolation; 2026-10-10T04-58-36Z_design_signed_artifacts; 2026-10-10T04-59-57Z_feature_cross_world_access; 2026-10-10T06-00-00Z_research_identity_capabilities; 2026-10-10T07-00-00Z_feature_petnames_invites
- 2026-10-11T19-04-32Z_phase_0_baseline; 2026-10-11T22-25-52Z_phase_1_events_storage
- 2026-10-12T04-03-47Z_phase_2_membership; 2026-10-12T08-58-59Z_phase_3_sync_relays; 2026-10-12T14-33-36Z_phase_4_groups_encryption; 2026-10-12T14-33-36Z_phase_5_tags_documents; 2026-10-12T19-11-31Z_phase_6_invites_pull_cli; 2026-10-12T21-22-53Z_phase_7_connectivity
- 2026-10-13T23-44-27Z_phase_8_open_design_items
- 2026-10-14T_platform_completion (+ Cloudflare Workers design, portable world surfaces, source-only world installs, keyboard parity, all 2026-10-14)
- 2026-10-15/16: roc_json, world viewport/paste, terminal buffer capture, world module provenance, resource profiles, capability ceilings, world CLI controls, roc_grid, roc_cap, chat history, records readback, world authoring CLI, guest reflow, screen widgets
- 2026-10-17/18: world roles directory, fixtures, scheduled events, next-stage platform architecture, perf budgets, territory view fuel fix, platform state reference/report

## Caveat recorded in the census

GitHub Search found zero merged PRs in developing-today/code for 2026-03-01 → 2026-09-27, so the March–April records are treated as design records carrying their original dates, not as a verifiable build history for those months.
