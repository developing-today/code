# Cost anomaly detector (#4825, partial)

The detector and warning-only intake are implemented and covered by local fixtures. **They have no production caller.** No route, startup hook, timer, environment setting, data reader, schema change, or notification class is added. The only intake is dependency-injected, and unless its caller explicitly enables it, it does not read evidence or call the alert store. This does not complete #4825.

## Evidence a future producer must certify

Use elapsed UTC calendar weeks (Monday–Monday) or months. Both sides of each comparison must cover the whole exact period. An empty cache, filtered Project view, absent observation or stale source does not certify a zero.

- **Burn without progress:** a complete existing spend rollup plus complete issue-closure history for the same epic/project, time window and attribution version. Consume one existing unit at a time; never blend tokens, PAYG dollars or plan-window percentage.
- **Single session:** a complete epic token rollup and the maximum root-session token count from that same pool, epic, window and attribution version. Include folded sub-sessions once, not as competing root sessions. Alert strictly above 50%.
- **Unattributed growth:** complete current and immediately preceding unattributed token rollups with complete denominators covering their same pools, periods and attribution versions. Each denominator is the total source population before attribution, not a sum of overlapping Project and epic rows. Alert strictly above a ten-percentage-point increase.
- **Charge without activity:** a complete, deduplicated monthly total of matched PAYG charges for one service, together with complete service-level session and quota coverage certifying zero activity in that month. Do not interpret lack of project attribution as inactivity; unmatched or estimated charges do not qualify.

The warning includes the scope, UTC interval, compared values and a link to the existing Projects or quota surface. The producer emits one authoritative fact per key; the intake does not reconcile conflicting raw observations that do not both qualify. One batch collapses identical warning evidence and suppresses conflicting warnings for the same key. Subsequent ticks use the ordinary alert ledger dedupe: an open incident retains first-seen evidence while its occurrence count and last-seen timestamp update. This intentionally does not alter alert-ledger semantics. In the existing store, warning incidents do not enter critical push delivery or escalation queues.

## Still needed

Production rollup ingestion and the certified auxiliary sources above are not wired by the current repository. Before activation, demonstrate those source contracts (including missing/partial-data cases), verify the source-to-rollup reconciliation, and verify warning presentation in Needs-you and Today/brief against the live application architecture without sending pushes. Do not turn on a producer merely because the detector tests pass.
