# Roadmap: Atlas

Atlas is the fleet's index: every corpus (conversations, memory files, notes, repository docs, research) brought in through connectors into one place to search, with a query surface for agents and people and standing evaluation sets that say whether it is any good. The search itself is a Rust binary and a service built on it (`pkgs/atlas-search`).

Written on 2026-11-23 from the programme's epics. A row's status is read from its issue's own labels on that day: an issue in review or in progress is `in-progress`, one filed with its steps and not started is `planned`, a design is `shaped`.

## Now

| Title                                                              | Status      | Intent                                                                                                                                                                           | Links                                                         |
| ------------------------------------------------------------------ | ----------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| The Atlas programme: one search and memory index over every corpus | in-progress | Every corpus ingested through a connector into one staging index, one way of embedding with each corpus's model named, a surface to query it, and evaluation sets as the oracle. | #8983, #5350                                                  |
| Atlas as a Rust binary and a multi-client search service           | in-progress | One binary that works alone and as a service. Its six steps, from embedder parity to the cutover behind a golden-parity gate, are closed; the epic stays open for what follows.  | #5350, #5351, #5352, #5353, #5354, #5355, #5356               |
| Backup mesh                                                        | in-progress | Every Turso database to R2 as fourteen days of deltas, copies on the laptop and in Drive, the D1 and laptop databases into Turso, and restores verified in rotation.             | #9093, #13455, #13456, #13458, #13463, #13457, #13459, #13461 |
| Shard consumer: serve mode and replicas that persist               | in-progress | The search role serves from shards, with embedded replicas kept between runs.                                                                                                    | #6666, #11213                                                 |
| A hot cache in D1                                                  | in-progress | Invalidation by tenant events is in; the measurement that says what it saves is in review.                                                                                       | #13106, #13107                                                |
| Metadata schema v1, and how it may change                          | in-progress | Additive only, with extras and strict enums.                                                                                                                                     | #9095                                                         |

## Next

| Title                                      | Status  | Intent                                                                                                                       | Links                          |
| ------------------------------------------ | ------- | ---------------------------------------------------------------------------------------------------------------------------- | ------------------------------ |
| Memory slice 3: the query surface over MCP | planned | Agents query the one index through the mcpx namespace.                                                                       | #8625                          |
| The projector for id                       | planned | Versions, files and relations, then attestations and ingest runs, its operations, and view presets for publishing the graph. | #11182, #11183, #11184 |
| Provenance registry                        | planned | Where each chunk occurs, extracts addressed by their content, and vectors resolved across databases.                         | #3125                          |
| Muse import                                | shipped | What counts as evidence of an import, how it is replayed, and when the old store is retired.                                 | #13607                         |
| The Atlas book                             | planned | Replay, scope and release protocols written up by workstream, with the module inventory checked as the book is built.        | #13592, #13563                 |

## Later

| Title                                | Status  | Intent                                                                                                                                          | Links |
| ------------------------------------ | ------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | ----- |
| The ingestion programme              | shaped  | Updates half-automated: what is ingested, how content is versioned, one model of provenance, deltas only, and feedback from what was retrieved. | #6018 |
| First-pass ingestion of every corpus | planned | Every code repository and every document that can be found.                                                                                     | #6025 |
| Cold tier: the restore importer      | planned | What was moved to cold storage can be brought back.                                                                                             | #8228 |
| Boundary remediation                 | planned | Rows that carry secrets leave the production store.                                                                                             | #8479 |
