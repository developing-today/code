# Turso estate — definitive database list (lane TURSO-SETTLE, programme #13900)

**Settled: 2026-11-23. TRUE TOTAL: 50 databases.**

**Source (route 1, definitive):** live `turso db list` on the laptop
(`turso` CLI v1.0.33 at `/etc/profiles/per-user/user/bin/turso`, already
installed and authenticated), run 2026-11-23 via `~/tools/laptop-ssh.sh`.
The command exited 0 and returned exactly 50 rows. Every database sits in
the single Turso group **`great-star-lord`** (the GROUP column for all 50
rows); hostnames are all `…-dezren39.aws-us-east-1.turso.io`.

**Cross-check (route 2):** the #13844 audit record
(`doc/2026-11-23T23-15-07Z_design_turso-d1-migration-audit/` in
developing-today/code, read on the laptop clone) names every non-Atlas
database individually; all 17 of those names match the live list exactly,
with none missing and none extra. The audit's method was a live Turso API
inventory read (2026-11-23 ~23:05Z); no token is recorded here — the audit
itself states tokens were fetched by name from the secrets broker and not
recorded.

## The full list (50, alphabetical, as returned)

Group for every entry: `great-star-lord`.

1. `atlas`
2. `atlas-ask-ledger`
3. `atlas-core-internal-agent-tooling-01`
4. `atlas-core-internal-agent-tooling-02`
5. `atlas-core-internal-agent-tooling-03`
6. `atlas-core-internal-agent-tooling-04`
7. `atlas-core-internal-cloudflare-01`
8. `atlas-core-internal-content`
9. `atlas-core-internal-fleet-repos-01`
10. `atlas-core-internal-fleet-repos-02`
11. `atlas-core-internal-fleet-repos-03`
12. `atlas-core-internal-home-lab-01`
13. `atlas-core-internal-home-lab-02`
14. `atlas-core-internal-infra-nix-01`
15. `atlas-core-internal-infra-nix-02`
16. `atlas-core-internal-media-01`
17. `atlas-core-internal-misc-01`
18. `atlas-core-internal-models-and-plans-01`
19. `atlas-core-internal-models-and-plans-02`
20. `atlas-core-internal-people-pets-01`
21. `atlas-core-internal-personal-admin-01`
22. `atlas-core-internal-personal-admin-02`
23. `atlas-core-internal-personal-admin-03`
24. `atlas-core-internal-personal-admin-04`
25. `atlas-core-internal-security-01`
26. `atlas-core-internal-work-engineering-01`
27. `atlas-core-internal-work-engineering-02`
28. `atlas-core-internal-work-engineering-03`
29. `atlas-core-internal-work-engineering-04`
30. `atlas-core-internal-work-engineering-05`
31. `atlas-core-internal-work-engineering-06`
32. `atlas-core-secret-content`
33. `atlas-core-secret-main-01`
34. `bedside-and-away`
35. `fleet-dispatch-jobs`
36. `fleet-mirror`
37. `fleet-mirror-recovery-stage-20261118`
38. `fleet-platform`
39. `fleet-platform-batched-stage-20261118`
40. `fleet-platform-reconcile-stage-20261118`
41. `fleet-platform-short-tx-stage-20261118`
42. `fleet-queue`
43. `fleet-supabase-recovery-20261117`
44. `fleet-supabase-runtime-legacy-stage-20261118`
45. `fleet-supabase-runtime-stage-20261118`
46. `fleet-supabase-runtime-tx-stage-20261118`
47. `great-star-lord`
48. `muse-meta`
49. `notes-submissions`
50. `voice-line-ledger`

## Reconciliation against the #13844 audit

The audit's **headline and group arithmetic were right: 50 estate-wide.**
Its Atlas group figure of **33 was also right.** Only its shard-family
figure — "30 `atlas-core-internal-*`" — was wrong, and that single wrong
figure is the whole 50-vs-52 discrepancy.

Audit groups, settled against the live list:

| Audit group                                                                                            | Audit figure | Live count | Verdict                                            |
| ------------------------------------------------------------------------------------------------------ | -----------: | ---------: | -------------------------------------------------- |
| Atlas                                                                                                  |           33 |         33 | ✅ correct total, wrong internal breakdown (below) |
| Fleet plane (`fleet-queue`, `fleet-platform`, `fleet-mirror`, `fleet-dispatch-jobs`)                   |            4 |          4 | ✅ exact                                           |
| Coordination (`muse-meta`)                                                                             |            1 |          1 | ✅ exact                                           |
| Small product stores (`notes-submissions`, `bedside-and-away`, `voice-line-ledger`, `great-star-lord`) |            4 |          4 | ✅ exact                                           |
| Frozen staging/recovery copies (the 8 named 2026-11-17/18 copies)                                      |            8 |          8 | ✅ exact, name-for-name                            |
| **Total**                                                                                              |       **50** |     **50** | ✅                                                 |

### Where the 52 came from — plainly

The audit's Atlas breakdown (answers-up-front and §G7) reads:
"`atlas` + 30 `atlas-core-internal-*` shards + 2 `atlas-core-secret-*`
shards, plus `atlas-core-internal-content` and `atlas-ask-ledger`".
Expanded literally that is 1 + 30 + 2 + 1 + 1 = **35**, i.e. 52 estate-wide.

The live Atlas set, counted non-overlapping, is **33**:

- `atlas` — 1
- `atlas-core-internal-*` content shards **excluding** the content store — **28**
  (agent-tooling ×4, cloudflare ×1, fleet-repos ×3, home-lab ×2,
  infra-nix ×2, media ×1, misc ×1, models-and-plans ×2, people-pets ×1,
  personal-admin ×4, security ×1, work-engineering ×6)
- `atlas-core-secret-*` — 2 (`atlas-core-secret-content`,
  `atlas-core-secret-main-01`)
- `atlas-core-internal-content` — 1
- `atlas-ask-ledger` — 1
- **Total: 1 + 28 + 2 + 1 + 1 = 33**

The overcount of exactly 2 is:

1. **`atlas-core-internal-content` double-counted.** It matches the
   `atlas-core-internal-*` glob — in the live list it is one of the 29
   names matching that glob (28 shards + itself) — and the audit then
   listed it _again_ as a separate "plus" item.
2. **A phantom 30th internal shard.** Even counting the glob generously
   (content included), the live family has **29** names, not 30. There is
   no 30th `atlas-core-internal-*` database; the audit's "30" figure is
   simply one too high on top of the double-count. (Equivalently, in
   non-overlapping terms: the true shard count excluding content is 28,
   not 30 — the "30" figure alone is wrong by 2.)

No database is missing from the audit, none is extra in the live estate,
and no other group figure changes. The estate total stands at **50**,
Atlas at **33**, covered/gap split in the coverage ledger unchanged
(34 covered / 16 gap).

— Lane TURSO-SETTLE, 2026-11-23. Coordinator: PM-BOOK (#13900).
