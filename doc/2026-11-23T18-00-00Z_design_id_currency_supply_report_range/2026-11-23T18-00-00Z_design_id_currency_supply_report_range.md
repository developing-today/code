# Supply reconciliation across the full ledger counter range

[Issue #13542](https://github.com/developing-today/code/issues/13542) corrects report arithmetic in the existing F4 audit from [#2320](https://github.com/developing-today/code/issues/2320).

Each account has unsigned 128-bit lifetime debit and credit counters. For each account, subtract exact counters before narrowing. Sum holder, fee and burn balances with arbitrary precision so ordering cannot change a verdict. The world feature uses the existing locked num-bigint 0.4.8 package for this reporting path.

Keep the existing signed 128-bit numeric report and mismatch shape for totals and differences that fit. For a completed read whose totals or difference exceed that range, return a distinct out-of-range finding with exact decimal issued, accounted and difference fields. This makes the consumer-visible limitation and the underlying discrepancy independently inspectable. Read failures continue to carry their existing finding. Production scheduling and ledger writes have separate owners.

The regression issues 2^127 + 1, moves one unit to an account outside the audited set, and checks that the report yields an exact one-unit shortage. Returning 2^127 to the issuer leaves small net amounts backed by large lifetime counters. Both stages exercise the regular reconciliation entry point.
