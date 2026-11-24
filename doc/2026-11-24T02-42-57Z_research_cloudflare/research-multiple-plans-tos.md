# Do Cloudflare's terms allow one person to buy multiple Workers plans?

Researched: 2026-10-20
Core answer based only on official Cloudflare legal pages and official Cloudflare developer documentation. Labelling used throughout:

- **(a) Legal text** — what the agreement actually says, quoted.
- **(b) Documented product behaviour** — what Cloudflare's official docs say the product does.
- **(c) Inference** — my reading / implication. Not legal advice. Where the terms are silent, that is stated plainly.

## Sources (exact URLs)

1. Self-Serve Subscription Agreement — https://www.cloudflare.com/terms/
   - Page heading on fetch: "Cloudflare Self-Serve Subscription Agreement", "Last Updated September 12, 2025"
2. Service-Specific Terms (overview) — https://www.cloudflare.com/service-specific-terms-overview/
3. Service-Specific Terms — Developer Platform — https://www.cloudflare.com/service-specific-terms-developer-platform/
   - Covers, per its own heading: "Cloudflare Workers; Cloudflare Pages; Workers KV; D1; Durable Objects; Vectorize; Hyperdrive; Cloudflare Queues; R2; Containers; and Workers for Platforms"
4. Workers pricing — https://developers.cloudflare.com/workers/platform/pricing/
5. Workers limits — https://developers.cloudflare.com/workers/platform/limits/
6. Create account — https://developers.cloudflare.com/fundamentals/account/create-account/
7. Organizations limitations — https://developers.cloudflare.com/fundamentals/organizations/limitations/
8. Organizations for Enterprise — https://developers.cloudflare.com/fundamentals/organizations/for-enterprise/

No Workers-specific standalone terms prohibiting multiple paid plans were found separate from (1) and (3) above. Workers is governed by the Self-Serve Subscription Agreement plus the Developer Platform Service-Specific Terms.

---

## 1. Is a Workers Paid plan per-account or per-user? Can a single account hold more than one Workers Paid subscription?

### (b) Documented product behaviour: per account

Workers pricing page, verbatim:

> "The Workers Paid plan includes Workers, Pages Functions, Workers KV, Hyperdrive, and Durable Objects usage for a minimum charge of $5 USD per month **for an account**. The plan includes increased initial usage allotments, with clear charges for usage that exceeds the base plan."

Source: https://developers.cloudflare.com/workers/platform/pricing/

The same page frames the included allotments per account per month:

> "10 million included per month +$0.30 per additional million" (requests)
> "30 million CPU milliseconds included per month +$0.02 per additional million CPU milliseconds"

And, for switching models:

> "To change your **default account-wide usage model**…"
> "Existing Workers will not be impacted when changing the default usage model. You may change the usage model for individual Workers without affecting your **account-wide default** usage model."

Workers limits page is headed "Account plan limits" and expresses limits per account, e.g.:

> "Number of Cron Triggers **per account** | 5 | 250" (Free | Paid)
> "Number of Workers | 100 | 500" under "Account plan limits"

Source: https://developers.cloudflare.com/workers/platform/limits/

### (c) Inference / silence

- The documentation treats "Workers Free" vs "Workers Paid" as a single plan attribute of **an account**, not a stackable quantity you buy in multiples for the same account.
- **No official page found describes buying two Workers Paid subscriptions on the same account to double the included allotment, and no dashboard/API mechanism for that is documented.** If you need more than the included amount on one account, the documented model is simply pay-as-you-go overage on that same account (e.g. $0.30 / million extra requests), not a second subscription.
- Therefore, in practice: *one account = at most one Workers Paid plan*. To get a second Workers Paid plan you need a second account (see Q2).

---

## 2. Can one person/company hold multiple Cloudflare accounts, each with its own Workers Paid plan? Do the terms restrict creating multiple accounts?

### Short answer

- **(b)** Cloudflare itself documents and supports one user creating additional accounts — multiple accounts are a normal product feature, not a hidden workaround.
- **(a)** The legal text does **not** contain a plain blanket sentence such as "you may only have one account" or "you may not buy a Workers plan on more than one account."
- **(a)** The legal text **does** prohibit two specific things that are directly relevant: (i) circumventing usage limits/quotas, and (ii) using software/automation to produce multiple accounts.
- **(c)** So a second *paid* account for a genuinely separate project/business purpose is not prohibited by any clause found; creating accounts specifically as a scheme to multiply free/included allotments, evade a limit, or farm free tiers is exactly the fact pattern the anti-circumvention clauses are written to catch — see quotes below.

### (a) Legal text — the two key clauses

Self-Serve Subscription Agreement, §2.2.1 Restrictions — https://www.cloudflare.com/terms/

§2.2.1(c):

> "(c) access or use the Services in a manner that violates or is intended to circumvent Service-specific usage limits, quotas, or other restrictions set forth in the Agreement;"

§2.2.1(e) — note carefully: this bans *automated* production of multiple accounts, not manual creation as such:

> "(e) introduce software or automated agents or scripts into the Services so as to produce multiple accounts, generate automated searches, requests or queries, or to strip or mine data from the Services;"

§2.2.1(a) — also relevant if accounts are being created for / sold to other people:

> "(a) rent, lease, loan, export, or sell access to the Services to any third party, or sign up for the Services on behalf of a third party;"

Consequence clause, §8 (same agreement):

> "If we determine you have breached Section 2.2 or 2.7, we may immediately Suspend or terminate all or part of your use of the Services, or limit End User access to certain of your resources through the Services."

Free-tier discretion, §2.6 (same agreement):

> "We may offer free or trial versions of the Services ("Free Services") from time to time… we will make each such Free Service available to you free of charge until the earlier of (a) the end of the free trial period (if applicable)…; (b) the start date of your Subscription Term for the Paid Services version of such Free Service; or (c) termination of the Free Service by Cloudflare in our sole discretion. **We will have no liability for any harm or damage arising out of or in connection with any Free Services.**"

Acceptable Use, §2.7 (same agreement), prohibits among other things facilitating "phishing, spamming, or other technical abuse" and illegal activity. It contains **no** clause about multiple accounts or multiple subscriptions.

### (a) Legal text — Developer Platform Service-Specific Terms

https://www.cloudflare.com/service-specific-terms-developer-platform/

Developer Platform §1:

> "Cloudflare may temporarily limit your storage and/or the number of requests you can make or receive using the Developer Platform if processing such requests would put an undue burden on the Cloudflare network, adversely impact the Service, or otherwise threaten the integrity of Cloudflare's networks."

Developer Platform §6:

> "The creation of Developer Platform accounts/projects using subdomains that include deceptive or offensive terms or names of other businesses, organizations, or individuals is prohibited."

No clause in the Developer Platform terms found that prohibits holding multiple accounts, multiple Workers plans, or splitting projects across accounts. The terms are **silent** on that specific point, beyond the general anti-circumvention and anti-automation clauses in the main agreement quoted above.

### (b) Documented product behaviour — multiple accounts are explicitly supported

"Create account" docs — https://developers.cloudflare.com/fundamentals/account/create-account/

> "## Create an additional Free account
>
> Existing users can create additional Free accounts in the dashboard or with the API.
>
> ### Eligibility
>
> - Your Cloudflare user must be active and at least seven days old.
> - You must have the Super Administrator role on an existing account.
> - **You can create up to five additional Free accounts.**"

The same page documents creating accounts via dashboard and via API (user-owned token / OAuth, with an Idempotency-Key), and lists an error for hitting the limit:

> "| You reached an account creation limit. | Use an existing account or contact your account team. |"

This is a documented, self-serve flow for *additional* accounts under the *same user* — which strongly cuts against reading the agreement as a one-account-per-person rule.

### (c) Inference — how the clauses likely apply to two paid accounts

Labelled inference, not a quote and not legal advice:

1. **Paying twice is not itself "circumvention" on the face of the text.** Each additional account pays its own $5 minimum and gets its own included allotment, and pays overage beyond that on that account. Nothing found says "one included allotment per natural person / per company, worldwide."
2. **Motive and mechanism matter under §2.2.1(c) and (e).** If the *purpose* of splitting is to avoid a per-account limit/quota that would otherwise apply to what is in substance one workload — e.g. sharding a single app across N accounts solely to harvest N × free daily limits, or spinning accounts with scripts/bots (§2.2.1(e)), or creating fresh accounts to reset a free trial / free tier — Cloudflare could characterise that as use "intended to circumvent Service-specific usage limits, quotas, or other restrictions," and §8 gives it a suspend/terminate remedy.
3. **Separate-purpose accounts are the low-risk pattern the product is built for:** separate projects, separate clients, dev/staging/prod, separate businesses — each with its own billing — are exactly the use case the "create an additional account" docs and the Organizations feature (Q4) describe. The terms do not require you to prove that purpose in advance, but they also do not grant a safe harbour for limit-farming.
4. **Do not use multiple *user identities* to evade the account-creation limits** in Q4 (e.g. five-account cap, seven-day rule). No clause quoted above says "one user per person" in those words, but doing that specifically to evade a documented creation limit falls squarely within the ordinary meaning of §2.2.1(c) circumvention — inference.
5. **Silence statement:** The Self-Serve Subscription Agreement, as fetched 2026-10-20, is **silent** on (i) a maximum number of *paid* accounts per person/company, (ii) whether affiliated companies may each buy Workers Paid, and (iii) any requirement to disclose common ownership of multiple self-serve accounts. Do not treat silence as permission for limit-evading schemes — the anti-circumvention clause is purpose-based, not count-based.

---

## 3. Practical differences: is usage pooled per account? Is getting multiple pools itself a violation?

### (b) Documented product behaviour: yes, pooled per account, not per user

From the pricing page (URL above):

- Included amounts reset monthly, per subscription/account: all included usage is stated "on a monthly basis"; D1 note on the same page: "Monthly included limits reset based on your monthly subscription renewal date, which is determined by the day you first subscribed."
- Aggregation is within the account, e.g. for D1 storage: "Storage is based on gigabytes stored per month, and is based on **the sum of all databases in your account**."
- Workers usage model is "account-wide" by default, overridable per-Worker within that account (quote in Q1).

Consequences that follow mechanically from (b):

| | One account, Workers Paid | Two accounts, each Workers Paid |
|---|---|---|
| Minimum charge | $5 / month | $5 + $5 = $10 / month |
| Included requests | 10M / month, pooled across all Workers in the account | 10M / month **per account**, separate pools |
| Included CPU | 30M CPU-ms / month, pooled | 30M CPU-ms / month per account |
| Workers per account | 500 | 500 per account |
| Cron Triggers per account | 250 | 250 per account |
| Billing / renewal date | One subscription | Two separate subscriptions, potentially different renewal dates |
| Data / bindings | KV, D1, R2, DO, Queues live in that account | Separate namespaces/databases/buckets per account; no automatic sharing |

Workers in different accounts do not share an included pool, and overage in one account is not offset by unused included usage in another.

### (a) / (c) Is having multiple pools a violation?

- **(a)** No clause found says "thou shalt have only one included pool." The existence of separate pools is simply how per-account billing works.
- **(c)** The violation risk is not *having* two pools, it is *why and how* you got them — see Q2 inference points 2 and 4. Two paid accounts for two real projects, each paying its own minimum: no clause found prohibits it. A fleet of accounts created by automation, or split for the sole purpose of defeating a per-account quota/free limit that would otherwise bind a single workload: §2.2.1(c)/(e) risk, plus §2.6 gives Cloudflare sole-discretion termination for Free Services.

---

## 4. Adjacent documented limits: how many accounts / zones in practice?

### (b) Additional Free accounts under one user (self-serve)

Source: https://developers.cloudflare.com/fundamentals/account/create-account/ (quotes in Q2)

- Up to **five additional Free accounts** per eligible user (in addition to the original account).
- User must be active and **at least seven days old**.
- User must have **Super Administrator** role on an existing account.
- API creation requires a user-owned token / OAuth (account-owned tokens cannot create accounts), `standalone: true`, and an Idempotency-Key.

No equivalent published self-serve number was found in the official docs fetched for "maximum paid accounts per user." The docs only document the Free-account creation flow and its cap; upgrading/creating a paid account is a billing/subscription action on an account. **Silence in docs ≠ no internal limit** — the error table itself acknowledges an "account creation limit" that can be hit.

### (b) Organizations (Enterprise / MSSP — not ordinary self-serve)

Source: https://developers.cloudflare.com/fundamentals/organizations/limitations/

> "Each Organization supports a maximum of **20,000 accounts** and **200,000 zones**. This limit applies to both Enterprise and MSSP/Distributor Organizations. These limits are defaults and can be adjusted for larger enterprises and partners in some cases."

Source: https://developers.cloudflare.com/fundamentals/organizations/for-enterprise/

> "One Organization contains multiple accounts… Maximum: **20,000 accounts** and **200,000 zones** per Organization"
> Example given by Cloudflare itself: "Company A is a SaaS company with **12 Cloudflare accounts**: 3 accounts for different product lines… 3 environments per product (Development, Staging, Production)"

That official example is useful context for Q2: Cloudflare's own documentation presents a single company holding a dozen accounts for product/environment separation as a normal pattern.

Prerequisite noted on that page: Organization creation requires an Enterprise plan account, Super Admin, and 2FA/SSO — so this is not a self-serve workaround for the five-account creation cap.

### (b) Per-account Workers limits that make a second account tempting — and why intent matters

Source: https://developers.cloudflare.com/workers/platform/limits/ ("Account plan limits")

- Workers per account: **100 (Free) / 500 (Paid)**
- Cron Triggers per account: **5 (Free) / 250 (Paid)**
- Free daily requests: **100,000/day per account** (pricing page / limits page: accounts on Free have a daily request limit of 100,000, Error 1027 on excess)

### Zones

- No hard self-serve per-account zone (domain) cap was stated in the official pages fetched, other than the Organization-level 200,000 zones above. That is a finding of *absence in the sources checked*, not a guarantee that no internal/anti-abuse zone limit exists. Do not cite a specific self-serve zone maximum from this file.

---

## Bottom line (one paragraph)

**(a)+(b)** A Workers Paid plan is a per-account subscription ($5/month minimum per account, one plan per account in the documented model; usage and included allotments pool per account). Cloudflare's own docs support one user creating additional accounts (up to five additional Free accounts self-serve, subject to age/role requirements), and each such account can carry its own paid subscriptions — no clause found imposes a one-plan-per-person rule. **(a)** What the Self-Serve Subscription Agreement does prohibit is (c) using the Services in a manner "intended to circumvent Service-specific usage limits, quotas, or other restrictions" and (e) using "software or automated agents or scripts… so as to produce multiple accounts," with suspension/termination as the remedy for breaching §2.2. **(c)** So: buying a second Workers Paid plan on a second, genuinely separate account is not prohibited by any text found; manufacturing/splitting accounts — especially automatically, or to farm free tiers / multiply included allotments for what is really one workload to dodge a limit — is the conduct those clauses target, and the terms give Cloudflare discretion to act on it. When in doubt, the safe, documented alternative for "I need more on this workload" is overage on the single account, Workers for Platforms, or asking Cloudflare Support / Sales for a limit increase in writing.
