# Free-services adoption record — #1059 (epic #1047)

Record date: 2026-11-03. Recorded by queue task 8044 (repo-worker-code, lane code-n).
Evidence baseline: the 2026-10-29 evaluation of the 11 services (issue #1059,
comment 6501568013; snapshots in the workspace evidence baseline §5,
`ai-subscriptions/evidence-and-interpretation-baseline-2026-10-20.md`). This record
publishes those verdicts and the adopt/skip dispositions into the repo, per epic
#1047's requirement that the dashboard link the research record. No account was
created, no card was added, no billing was enabled, and no billable resource was
created for this record.

Standing rule applied (from the workspace tracker): under the no-surprise-bills
guardrail, a service is not adopted unless its overage verdict is a wall (or it is
already in use). "Skip" below means _not adopted as of 2026-11-03_; it is a
disposition of record, not a ban — a later adoption is a deliberate follow-up
(master-tracker row + hub card via #1047's children) once Drew rules on it.

## Adopt/skip rows (11)

| Service      | 2026-10-29 verdict                     | Disposition 2026-11-03               | Reason (one line)                                                                                                                                                                                                                    |
| ------------ | -------------------------------------- | ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| boats.cloud  | unverified                             | skip — unverifiable                  | No official boats.cloud site, pricing, or docs exists or resolves; provider identity required first (recorded unverifiable, not left open).                                                                                          |
| quic.cloud   | wall                                   | skip — adopt candidate, pending Drew | CDN/DNS free tier walls (quotas pause at exhaustion, extra only via prepaid credit); passes the guardrail on paper, but adoption is a follow-up decision, and the announced 2026-11-10 billing-model change must be rechecked first. |
| Backblaze B2 | excess-bills (verified)                | skip — adopt candidate, pending Drew | First 10 GB free with no card is usable as a backup target; adding a payment method is the decision point (alerts/caps first), so no silent adoption.                                                                                |
| OVHcloud     | excess-bills                           | skip                                 | No permanent free tier; one-time trial voucher requires a payment method upfront and auto-bills after credit; postpaid with no hard cap.                                                                                             |
| Railway      | excess-bills (paid plans)              | skip                                 | Permanent free is a $1/mo credit; usage past included amounts bills the delta and the hard limit is opt-in — fails the wall test.                                                                                                    |
| Render       | wall (no payment method)               | skip — not adopted                   | Walls only while no card exists; free Postgres expires (30 days + 14-day grace), so it is never durable data — no current need recorded.                                                                                             |
| DigitalOcean | excess-bills                           | skip                                 | No permanent free compute tier; trial requires a card and overage charges it; resources keep billing after the trial.                                                                                                                |
| Hetzner      | excess-bills                           | skip                                 | No free tier or trial found; hourly billing with traffic overage, cost alerts are explicitly not a spending limit.                                                                                                                   |
| Heroku       | wall (Eco, paid)                       | skip                                 | No free tier since 2022-11-28; cheapest app+DB is $10/mo paid — a spend decision, not a free-tier adoption.                                                                                                                          |
| Google Cloud | excess-bills (Paid account)            | skip                                 | Paid accounts bill past free-tier limits at standard rates; see guardrail record below — no use until the billing-disable automation exists and is tested.                                                                           |
| AWS          | wall (Free Plan) / excess-bills (Paid) | skip — adopt candidate, pending Drew | The Free Plan is a true wall for evaluation use, but starting it begins a 6-month / credit-exhaustion clock and enrolling in AWS Organizations silently auto-upgrades to Paid — the start must be deliberate.                        |

Provenance: verdicts and wording snapshots — issue #1059 comment 6501568013
(2026-10-29, queue task 2996) and the workspace evidence baseline §5 named above;
dashboard provenance summary — `pm-stack/aggregator/src/project-config.ts`
("Free-services pass — adoption decisions" entry, updated by the same change as
this record).

## GCP / AWS guardrail record (dated 2026-11-03)

No billable resources were created to verify any of this, and no console state is
claimed beyond what is written here. As of 2026-11-03 there is **no repo-visible
evidence** that either cloud's guardrails are configured, and neither cloud is in
use by the fleet (the 2026-10-29 evaluation created no account and enabled no
billing). The guardrail state of record is therefore:

- **Google Cloud — NOT CONFIGURED / NOT VERIFIED in any console.**
  Required before any use, configured in Google Cloud Console → Billing →
  Budgets & alerts (and the billing-account settings): a budget alert on the
  Google AI Pro $10/mo credit envelope, plus the automatic billing-disable
  automation (programmatic budget notification → Pub/Sub → function that
  disables billing, tested) — or billing not enabled at all. Budget alerts alone
  do not cap spend (Google's own docs say budgets "do not enable or prevent
  spending"), so an alert without the disable automation does not satisfy the
  guardrail. Verification date for this "not configured" state: 2026-11-03
  (repo + issue-record search by queue task 8044; console not accessed).
- **AWS — NOT CONFIGURED / NOT VERIFIED in any console.**
  Required before any Paid use, configured in AWS Billing and Cost Management →
  Budgets / Free Tier usage alerts (and IAM): Free Tier usage alerts (85%
  emails), a zero-spend budget, and minimal IAM. On the Free Plan no guardrail
  configuration exists yet because no account has been started; the plan's wall
  holds only while the account is not enrolled in AWS Organizations. Verification
  date for this "not configured" state: 2026-11-03 (repo + issue-record search
  by queue task 8044; console not accessed).

When either cloud is first used, this section is updated with the configuration
location, the configuration date, and the verifying read — dated evidence, never
an inference from prior comments.
