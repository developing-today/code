# Atlas Layer 1 classification revalidation — scan report 2026-11-15

Issue: developing-today/code#6021 · Executed by: queue task 16845 (repo queue worker, lane any-a) · Parent program: #6018 · Method ruling: #6050 (gitleaks-first) · Cadence design: #6051

**Handling:** this report contains counts, finding classes, and DB + chunk/message IDs only. No scanned content is reproduced anywhere in this report. The scan ran fully locally on the laptop; gitleaks ran with redaction on private export copies that were deleted immediately after each store was scanned.

## Method

- Scanner: gitleaks 8.28.0, default rule set, `detect --no-git --redact`, fully local (no verification calls of any kind; trufflehog not used, per #6050).
- Access: read-only tokens minted per DB via the turso CLI, held in memory only; chunk/message text streamed over the libSQL HTTPS pipeline in ID order, in batches of 1,000.
- Units: every chunk in every in-scope DB, plus every message in the two content stores (a second, full-text view of the same corpus). Per-store summaries (rule + unit ID only) were retained; all text exports and raw gitleaks reports were destroyed after each store.

## Tier shape — re-verified at run time (2026-11-15T00:36–00:50Z)

| Store group                                            | DBs     | Units                                                                                 | Notes                                                                                                                                           |
| ------------------------------------------------------ | ------- | ------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| Internal chunk shards (`atlas-core-internal-*`)        | 28      | 107,001 chunks                                                                        | matches the #6021 brief exactly; sensitivity labels: local-only + personal only — **zero chunks labelled secret-bearing** in any internal shard |
| Internal content store (`atlas-core-internal-content`) | 1       | 70,987 messages                                                                       | sessions: 2,226 local-only + 16,009 personal; no secret-bearing sessions                                                                        |
| Secret chunk shard (`atlas-core-secret-main-01`)       | 1       | 638 chunks                                                                            | matches the brief exactly (1 shard / 638 chunks)                                                                                                |
| Secret content store (`atlas-core-secret-content`)     | 1       | 564 messages                                                                          | 90 sessions, all labelled secret-bearing                                                                                                        |
| Production monolith (`atlas`, publish copy)            | 1       | 107,639 chunks                                                                        | = 107,001 + 638 exactly; sensitivity: 100,060 personal + 6,941 local-only + **638 secret-bearing**                                              |
| Local replica (`~/atlas/turso-atlas/data/atlas.db`)    | 1 file  | 107,639 chunks                                                                        | live local replica of the monolith; scanned separately (below)                                                                                  |
| Local publish copy (`atlas-publish-v2-2026-10-31.db`)  | 1 file  | 107,639 chunks                                                                        | sensitivity split verified identical to the monolith (638 secret-bearing)                                                                       |
| Older local stores (inventory only)                    | 3 files | v3/corpus.db 101,471 chunks; atlas-delta-chunks.db 55,098; atlas-2026-10-26.db 39,432 | pre-publish canonical/snapshot stores; sensitivity labels for these live in the publish DB, whose full set was scanned                          |

Total units scanned this round: **394,468** (286,829 remote + 107,639 local replica).

## Results by store view

| View                              | Units scanned | Findings | Distinct units with findings | Classes                                                                                                                        |
| --------------------------------- | ------------- | -------- | ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| Internal chunk shards             | 107,001       | 67       | 47 chunks                    | generic-api-key 56, jwt 7, private-key 1, github-oauth 1, curl-auth-user 1, curl-auth-header 1                                 |
| Internal content store (messages) | 70,987        | 83       | 45 messages                  | generic-api-key 72, jwt 7, private-key 1, github-oauth 1, curl-auth-user 1, curl-auth-header 1                                 |
| Secret chunk shard                | 638           | 4        | 1 chunk                      | jwt 4                                                                                                                          |
| Secret content store (messages)   | 564           | 4        | 1 message                    | jwt 4                                                                                                                          |
| Monolith (remote)                 | 107,639       | 71       | 48 chunks                    | secret-bearing group: jwt 4 (exactly the secret-tier material); all other groups: 67 — exactly the internal-tier candidate set |
| Monolith (local replica)          | 107,639       | 71       | 48 chunks                    | identical class distribution to the remote monolith                                                                            |

Per-shard internal results (chunks scanned / findings):

| DB                                      | chunks | findings |
| --------------------------------------- | ------ | -------- |
| atlas-core-internal-agent-tooling-01    | 3866   | 1        |
| atlas-core-internal-agent-tooling-02    | 3866   | 2        |
| atlas-core-internal-agent-tooling-03    | 3866   | 0        |
| atlas-core-internal-agent-tooling-04    | 3866   | 0        |
| atlas-core-internal-cloudflare-01       | 1250   | 0        |
| atlas-core-internal-fleet-repos-01      | 3458   | 7        |
| atlas-core-internal-fleet-repos-02      | 3457   | 10       |
| atlas-core-internal-fleet-repos-03      | 3457   | 0        |
| atlas-core-internal-home-lab-01         | 4070   | 1        |
| atlas-core-internal-home-lab-02         | 4069   | 0        |
| atlas-core-internal-infra-nix-01        | 4094   | 2        |
| atlas-core-internal-infra-nix-02        | 4094   | 0        |
| atlas-core-internal-media-01            | 1607   | 0        |
| atlas-core-internal-misc-01             | 841    | 0        |
| atlas-core-internal-models-and-plans-01 | 4551   | 1        |
| atlas-core-internal-models-and-plans-02 | 4551   | 0        |
| atlas-core-internal-people-pets-01      | 4502   | 0        |
| atlas-core-internal-personal-admin-01   | 4360   | 5        |
| atlas-core-internal-personal-admin-02   | 4360   | 2        |
| atlas-core-internal-personal-admin-03   | 4360   | 0        |
| atlas-core-internal-personal-admin-04   | 4359   | 0        |
| atlas-core-internal-security-01         | 3489   | 13       |
| atlas-core-internal-work-engineering-01 | 4435   | 3        |
| atlas-core-internal-work-engineering-02 | 4435   | 7        |
| atlas-core-internal-work-engineering-03 | 4435   | 1        |
| atlas-core-internal-work-engineering-04 | 4435   | 11       |
| atlas-core-internal-work-engineering-05 | 4434   | 1        |
| atlas-core-internal-work-engineering-06 | 4434   | 0        |

## Reconciliation

- Monolith total (107,639) = internal shards (107,001) + secret shard (638), exactly.
- Monolith findings in non-secret sensitivity groups (67) = internal-shard findings (67), exactly; monolith findings in the secret-bearing group (4 jwt) = secret-shard findings (4 jwt), exactly.
- Local replica findings (71) = remote monolith findings (71) with an identical class distribution.
- The message view finds more (83) than the chunk view (67) on the same corpus: full message text also catches matches that span chunk boundaries. The chunk list is canonical for remediation; the message list is the catch-net (both are attached to follow-on #10412).

## Converse boundary — where secret material sits

Verified this round:

1. Labelled secret-bearing chunks exist in exactly one chunk shard: `atlas-core-secret-main-01` (638). No internal shard carries a secret-bearing label (checked per shard, all 28).
2. Secret-tier content-store sessions (90, all secret-bearing) exist only in `atlas-core-secret-content`; the internal content store has none.
3. The publish monolith and its local copies do carry the same 638 secret-bearing chunks — by design: they are labelled secret-bearing there and the atlas-search query layer gates them behind the explicit `--max-sensitivity secret-bearing` ceiling (default ceiling `personal`). Their credential-shaped findings are exactly the secret tier's own 4 jwt findings — the publish set contains no additional credential-shaped material beyond the tier stores.
4. The secret tier's own findings (jwt x4 in chunk 22599 of `atlas-core-secret-main-01`; jwt x4 in message 227d556b125058100747046605c10961b99897a51a3d487b5cfb179f9dc52d5c of `atlas-core-secret-content`) are the same material in its correct tier — no action.

**Conclusion:** no labelled secret material sits outside the two secret DBs and the gated publish copies of them. The open question is the candidate set below: credential-_shaped_ material in the internal tier whose labels say personal/local-only.

## Candidate findings — internal tier (NOT confirmed mis-tiers)

gitleaks-class rules fire on code, config examples, fixtures, and discussion text as well as on real credentials. These 47 chunks are candidates for the secret-tier review filed as **#10412** (queue task 17341): each will be ruled confirmed (→ move + re-embed at secret tier, purge from internal incl. replicas) or dismissed (false positive), at secret-tier sensitivity, IDs + ruling only.

| DB                                      | chunk ID | classes            |
| --------------------------------------- | -------- | ------------------ |
| atlas-core-internal-agent-tooling-01    | 99509    | github-oauth       |
| atlas-core-internal-agent-tooling-02    | 101449   | generic-api-key    |
| atlas-core-internal-agent-tooling-02    | 110522   | curl-auth-user     |
| atlas-core-internal-fleet-repos-01      | 98814    | generic-api-key x3 |
| atlas-core-internal-fleet-repos-01      | 98815    | generic-api-key x4 |
| atlas-core-internal-fleet-repos-02      | 52104    | generic-api-key    |
| atlas-core-internal-fleet-repos-02      | 99190    | generic-api-key    |
| atlas-core-internal-fleet-repos-02      | 99255    | generic-api-key    |
| atlas-core-internal-fleet-repos-02      | 99256    | generic-api-key x6 |
| atlas-core-internal-fleet-repos-02      | 99272    | generic-api-key    |
| atlas-core-internal-home-lab-01         | 76475    | generic-api-key    |
| atlas-core-internal-infra-nix-01        | 50613    | generic-api-key    |
| atlas-core-internal-infra-nix-01        | 78013    | generic-api-key    |
| atlas-core-internal-models-and-plans-01 | 99285    | generic-api-key    |
| atlas-core-internal-personal-admin-01   | 112901   | jwt                |
| atlas-core-internal-personal-admin-01   | 112910   | jwt                |
| atlas-core-internal-personal-admin-01   | 112951   | curl-auth-header   |
| atlas-core-internal-personal-admin-01   | 112978   | generic-api-key x2 |
| atlas-core-internal-personal-admin-02   | 110005   | generic-api-key    |
| atlas-core-internal-personal-admin-02   | 6404     | generic-api-key    |
| atlas-core-internal-security-01         | 100103   | generic-api-key    |
| atlas-core-internal-security-01         | 14052    | jwt                |
| atlas-core-internal-security-01         | 20305    | generic-api-key    |
| atlas-core-internal-security-01         | 22892    | jwt x3             |
| atlas-core-internal-security-01         | 22903    | private-key        |
| atlas-core-internal-security-01         | 23247    | generic-api-key    |
| atlas-core-internal-security-01         | 40003    | generic-api-key    |
| atlas-core-internal-security-01         | 40021    | generic-api-key x3 |
| atlas-core-internal-security-01         | 40022    | generic-api-key    |
| atlas-core-internal-work-engineering-01 | 100830   | generic-api-key    |
| atlas-core-internal-work-engineering-01 | 100842   | generic-api-key    |
| atlas-core-internal-work-engineering-01 | 46868    | generic-api-key    |
| atlas-core-internal-work-engineering-02 | 100871   | generic-api-key    |
| atlas-core-internal-work-engineering-02 | 17351    | jwt                |
| atlas-core-internal-work-engineering-02 | 21773    | generic-api-key    |
| atlas-core-internal-work-engineering-02 | 22274    | generic-api-key x2 |
| atlas-core-internal-work-engineering-02 | 22275    | generic-api-key    |
| atlas-core-internal-work-engineering-02 | 22276    | generic-api-key    |
| atlas-core-internal-work-engineering-03 | 46875    | generic-api-key    |
| atlas-core-internal-work-engineering-04 | 25250    | generic-api-key x2 |
| atlas-core-internal-work-engineering-04 | 25257    | generic-api-key x2 |
| atlas-core-internal-work-engineering-04 | 25259    | generic-api-key    |
| atlas-core-internal-work-engineering-04 | 25260    | generic-api-key    |
| atlas-core-internal-work-engineering-04 | 25266    | generic-api-key x2 |
| atlas-core-internal-work-engineering-04 | 25269    | generic-api-key x2 |
| atlas-core-internal-work-engineering-04 | 69455    | generic-api-key    |
| atlas-core-internal-work-engineering-05 | 13041    | generic-api-key    |

Highest-priority candidates: chunk 22903 in atlas-core-internal-security-01 (private-key); chunk 22892 in atlas-core-internal-security-01 (jwt x3); chunks 112901 + 112910 in atlas-core-internal-personal-admin-01 (jwt); chunk 99509 in atlas-core-internal-agent-tooling-01 (github-oauth).

Candidate messages in `atlas-core-internal-content` (83 findings, 45 messages):

| message ID                                                       | classes            |
| ---------------------------------------------------------------- | ------------------ |
| 045ab46f6a1a855ae7f8210c7f27dd5226e85ba4abbbeef3bfbd80f9efab2a6a | generic-api-key    |
| 1e16bf17f17f4b8f6277ad4614978d223d8e2fc4cff8608e03ab917b738572e5 | generic-api-key    |
| 28b218c9b3495e310fa0f7ff11a8885b0cead905cda6225e04e91b19424368a6 | generic-api-key    |
| 3c80c7ade224a2906b44c342359e4bf0e02aa99522d817fcef5444fb04944588 | private-key        |
| 4a097fb446ad179982f4e663f45b0cdf4419746ee85d1659013c79a10381da26 | generic-api-key    |
| 4e0b2474f6fb82e6b438424ea18d76b5778f824094f35c0c4988f1d4bac60e93 | generic-api-key x2 |
| 4f88440b4f3a7859376cbee97fadfac774aa019d0d8b5c2fb25134bab3482bdb | generic-api-key    |
| 58bba91482888f404a6a21c38e655e99be691eef93265f951e4b170742e8329f | generic-api-key    |
| 5ed809f187def49217bc301d4f224f23cf53116b727a1aa71a923f16508a0ac8 | jwt                |
| 606f32d8da3e737fee35d75e0d40f00d5ab6e9b0680932012c2b4885a99f1e93 | curl-auth-header   |
| 62c8215d08645c8c04dae190508baca05ea6481d5e1a38e8272597a1687b19e6 | generic-api-key    |
| 695c15c6804e55471808d74a33cade1ae54076fd14182da620b01ebb51a92fff | curl-auth-user     |
| 70b849dec612afb1fd53bc68ef985ba980d8d00f81c2686df9b516ffabddba4a | generic-api-key    |
| 761a83f96bbcd6f84455c20664b2c2221fdc3ed7d68792d2f1d9596953f19316 | generic-api-key    |
| 77677f6e64e38e9d1e7be406238949f8c80d28be810a03e09bf8ee7441fbb072 | generic-api-key x2 |
| 7a4d63e72a194b73171c51ad6ee45bfef758a2ef9650d4ebbbcdf45b6402b854 | github-oauth       |
| 8238b4ce7f9521e3381f2a169e47acbbf921dc472f51fcc02239b049e7fb958a | generic-api-key    |
| 83d1d207fcc12c3829f8d139488182cd27749d01b814e2751bb472c845c4f442 | generic-api-key x2 |
| 8ec9a43e4654d13fef06f780439b467bec9070bc0090746b2e7eb65fc5376e75 | generic-api-key    |
| 92490708252eb89e0f6dc2f26fcf8248fea198b92f9d2d5125bce77092aa8bc0 | generic-api-key    |
| 9c91aab8452df0c1a2a507ac59ee5340b6f43a6bdd91efdb05d27dc0c52e32a4 | generic-api-key x2 |
| 9e6e89ab679367bdcf1ca3504847200247386b84b0924d39a89670941ce6b101 | jwt x3             |
| a4afd4dda1b87911cde77e5649ead3fd6cd52cc3c00cd1342d82af423d0352f2 | generic-api-key x8 |
| a88c6bc799352d7f74205888f23adaa452f7dc09a490059ffb60189bf8a2ea6a | generic-api-key x2 |
| a8ce6ded065d841fa8ea6a340113e4e8ef792122481b8d862f2a31cfc71e45f6 | generic-api-key    |
| ab83b51ce3d1112009fbb12cb09c843b24141a07074f87dec31b3ee5759fb66a | generic-api-key x7 |
| ad7727b37fa52954fab68282fda5859dd57f7a76e10bdef18eb461d141a10979 | generic-api-key x7 |
| ad79a6747410d0b56b689d793e313d99990f0226deec85fd659fad9e077c2e61 | generic-api-key    |
| b2d5b84eea3d8ee9c5ac5589ce4cee749c34ed6b20fa5b569dad56dfae1eef83 | generic-api-key x8 |
| b4517d9475564bd6a4ff95cc068a6df0bbf3a8b27b05a84c8c1423e8ab64d1ee | generic-api-key    |
| b9b6cb3d1e206498d4fe60dc7c43f5f3006c0fb32e975886b16e789d949ca3e0 | jwt                |
| baeddbe1950a329c9117e9646678053614992daf7f1e3b9d1cab35ee8a9383ca | jwt                |
| bce5c9a9b803859182859170de2a483cb4b22ce4d3451a0976ecfa09fa52132e | generic-api-key x2 |
| c2a215883c7b8aa21b841bc102a6a8af6f1f37cafaa8b90870cb7b653eeab2c6 | generic-api-key    |
| c73a5808aa7bd12045aa18520efb380c24e2073cc031072fa5d9fab8455a8c4a | jwt                |
| d3e24d96bfc13efd18ad9d8fba9dfc1a79855308da2621563333ec5684d977f7 | generic-api-key x2 |
| d9e92056877cfdacc89ccd821c0af65e9708e413e3a4175ddc3a8b2cff7d3c41 | generic-api-key x4 |
| da5b883b8c81070f8c1234d49c4e637b37e2726d82cf507ea55da91a0a2674aa | generic-api-key    |
| e1ca1e3e9e16b7ed15dadc5a9580130084cc216dd644002973df5a96818ee450 | generic-api-key    |
| e39cfdc16c99d53ebf621c3e2cfa97d1dfb87a6bffcbbca37be15cee36098cc6 | generic-api-key    |
| e97b1e5e92c6bf92afd9185cdc8a9fd4a2885e2fce65b68e4255a25e3149b12f | generic-api-key    |
| ebbed4a21ba546760f2b183c62ca3403962b8d22874154c32cee33d9f677ef18 | generic-api-key    |
| eeebc17faec200004fdbefc3a585452c1d8e5e2dfc43d4163dcf3ff9f57b5531 | generic-api-key    |
| f5dc2fb4de56e179de44133553b318d2b7ed57388d10b187b34a822c9b87a5a3 | generic-api-key    |
| fb4fe992bb93b57aab39a8009644fdcbc1b15196af5db410b7b18af02753426a | generic-api-key    |

## Recurrence recommendation (input to #6021 close-out under #6018)

Adopt the #6051 cadence as a standing gate, with this round as the baseline: candidate rate 67 findings / 107,001 internal chunks (47 chunks, 0.044% of the tier), 0 confirmed mis-tiers yet (review pending in #10412). A standing full L1 round per the cadence triggers (calendar T4 + rule/source/growth triggers T1–T3) is cheap at this scale — this full round scanned 394,468 units in ~15 minutes of laptop time — and the per-shard checkpoint shape used here (one summary per DB, idempotent resume) is directly reusable as the round runner. Recommend: standing gate YES, runner = this scan shape, baseline = this report.

## Scope notes

- Layer 1 only, per the packet: Layer 2 (Presidio NER, #6048 design) and Layer 3 (personal-inventory pattern pack, #6049) are separate waves and were not run.
- Older local stores (v3/corpus.db, atlas-delta-chunks.db, atlas-2026-10-26.db, staged-*.db) were inventoried, not scanned: they are pre-publish canonical/snapshot stores whose content feeds the publish set; the live replica and publish copy — the stores actually queried — were scanned in full.
- Staging-tier deep audit (#5669) is out of scope per the packet.
