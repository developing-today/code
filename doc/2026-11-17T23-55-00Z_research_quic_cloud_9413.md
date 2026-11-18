# QUIC.cloud current free tier and billing recheck (#9413)

**Checked:** 2026-11-17. **Scope:** official-source research only. No account,
resource, payment method, or trial was created or changed.

## Finding

**Recommendation: skip for the fleet.** QUIC.cloud provides WordPress CDN,
authoritative DNS, and WordPress optimization services; it does not host sites
or provide general-purpose compute. Its free CDN/DNS may be useful for an
existing WordPress origin, but no such fleet use is identified in #9413 or its
linked evidence. This is therefore not a way to add compute of various kinds.
Revisit only if the fleet has a concrete WordPress site needing these services.

## What changed, and when

The earlier record (`doc/2026-11-03T18-30-00Z_record_free_services_adoption.md`,
row 23) says a billing-model change was announced for 2026-11-10 and asks for a
recheck. The first-party material is inconsistent about that date: the CDN
pricing page, last dated November 5, still says billing is being simplified on
November 10, but its prominent notice says current pricing is valid through
November 30 and simplified billing is coming December 1. The plan-based billing
page, also dated November 5, and the provider's November 5 announcement both
say the new plans go live **December 1, 2026**. I found no first-party statement
that the new plan billing actually took effect on November 10. Treat November
10 as stale/inconsistent wording, not as the effective date.

Thus, **as checked November 17, the old credit/quota and CDN Free/Standard
model is still the current model; the unified plans are announced for December
1, not yet effective.** The provider says its current pricing is valid through
November 30.

## Free offerings and their fit

The official CDN pricing documentation (dated November 5, 2026) describes the
current Free Plan as unlimited bandwidth across six floating PoPs (three in
North America and three in Europe); Standard has a larger global network and
paid, regional bandwidth. The current Free Plan has basic security rather than
configurable DDoS/security features. The same doc says QUIC.cloud is a reverse
proxy: the site/domain points to its CDN, which fetches from the site's origin.

The official billing/online-services documentation (dated November 5) and FAQ
(dated October 5) describe the current free optimization quota, changed on
October 6: 100 image optimizations, 20 page-optimization requests, and 20 LQIP
requests per month, subject to daily limits for free domains. These are
WordPress optimization services, not hosted functions or general compute.

QUIC.cloud's official DNS page advertises free authoritative DNS. The dated
FAQ (October 5) also describes this as a free built-in DNS option. Using it for
CDN may require switching nameservers/delegating DNS; the cited materials do
not specify a DNS quota or a guarantee that the offer is permanent, so neither
is assumed here.

The FAQ (October 5) states: “QUIC.cloud was built exclusively for WordPress
sites” and “does not host your site.” It describes CDN plus online optimization
services. Consequently the compute shape is **none**: this is an edge
reverse-proxy/DNS/optimization layer in front of a separately hosted
WordPress site, not a VM, container, function, or app host.

## Billing and overage verdict

### Current model (through November 30)

The November 5 billing docs say services use quota/bandwidth bought with
account-level credit; purchased credit does not expire and is not automatically
allocated to a service. The FAQ says users can choose Auto-Refill to avoid
manually refilling low service balances. Thus the free quota by itself is
bounded, but the account can deliberately add a prepaid continuation path;
Auto-Refill is an additional setting that must not be presumed off for an
account the worker cannot inspect. Current Free CDN bandwidth is advertised as
unlimited, so it has no published bandwidth exhaustion point on that plan.

### Announced plan model (from December 1)

The November 5 plan-based billing page lists a $0 Free plan with unlimited CDN
bandwidth on the smaller US/EU network and 100 optimization units. It says
domains default to Free unless the owner chooses another plan. If CDN allowance
(plus packs) is exceeded, the domain moves to the smaller US/EU network for the
rest of the billing period; if optimization allowance (plus packs) is used,
optimization pauses until renewal. Packs are separately purchased for $20 and
add 250 GB CDN bandwidth or 10,000 optimization units; the page says they do
not expire. The plan doc also says an existing domain-level Pay-As-You-Go
balance remains and is used after the plan allowance. It does not state that
an exhausted free allowance automatically charges a card.

**Verdict: wall/throttle on free-only usage; not excess-bills under the
documented plan rules, provided no paid plan, pack, prepaid PAYG balance, or
Auto-Refill is enabled.** Optimization stops; CDN service is reduced to the
smaller US/EU network rather than billed as overage. Prepaid credit/PAYG is a
user-authorized spending path, not an unlimited postpaid overage: usage can
consume that finite balance, and the docs say packs are manually added. However,
the provider's plan doc explicitly says an existing PAYG balance is consumed
after allowance exhaustion, and the current FAQ exposes Auto-Refill. Do not
describe the setup as an unconditional wall without checking those settings.
No dashboard/account was accessed, and the plan FAQ does not establish whether
Auto-Refill carries over to the new plan model; that remains unverified.

## Recommendation

**Skip QUIC.cloud for now.** The billing change does not reveal a compute
offering; it clarifies CDN/WordPress optimization limits and appears to retain
a prepaid continuation path for existing PAYG credit. There is no identified
fleet WordPress origin, and DNS/CDN adoption would add a domain-routing
dependency without meeting the stated goal of simple fleet compute. If Drew
later identifies a WordPress origin that needs CDN/DNS, treat it as a new
deliberate adoption decision and verify credit, PAYG, and Auto-Refill settings
before use. No tracker row or hub card is proposed now; if adopted later, those
belong to children of epic #1047, not this issue.

## First-party sources (all accessed 2026-11-17)

1. QUIC.cloud, [Plan-Based Billing](https://docs.quic.cloud/billing/plan-based/),
   dated November 5, 2026 — announced December 1 effective date, plan table,
   included allowances, allowance exhaustion, packs, retained PAYG balance,
   and current free quota.
2. QUIC.cloud, [CDN Plans and Pricing](https://docs.quic.cloud/billing/cdn/),
   dated November 5, 2026 — current Free/Standard distinction, six PoPs,
   unlimited Free bandwidth, the conflicting November 10 sentence, current
   pricing through November 30, and credit mechanics.
3. QUIC.cloud, [Online Services Pricing](https://docs.quic.cloud/billing/services/),
   dated November 5, 2026 — current quota/credit mechanics and prepaid
   account-level balance.
4. QUIC.cloud, [Frequently Asked Questions](https://docs.quic.cloud/faq/),
   dated October 5, 2026 — WordPress scope, no hosting, free built-in DNS,
   CDN behavior, and Auto-Refill option.
5. Lisa Clarke, QUIC.cloud, [“Get Ready for Simpler Billing!”](https://www.quic.cloud/get-ready-for-simpler-billing/),
   published October 6, updated November 5, 2026 — confirms December 1 start,
   free plan default, future allowance-exhaustion behavior, and existing PAYG
   and account-credit handling. The November 16 comments clarify pack/security
   behavior; they are supplemental, not the basis for the billing verdict.
6. QUIC.cloud, [DNS Service](https://www.quic.cloud/quic-cloud-services-and-features/dns-service/),
   accessed November 17, 2026 — advertises “100% FREE Authoritative DNS
   service”; page has no publication date visible, so the dated FAQ above is
   the dated corroboration used for the free built-in DNS claim.

## Limits of this research

No account or dashboard was inspected. The DNS service's exact quotas and
permanence, Auto-Refill behavior after December 1, and any per-account billing
settings are not verified. The CDN page's November 10 text conflicts with the
same page's December 1 banner and the dated plan-based page and announcement;
the documented effective date used here is December 1, 2026.
