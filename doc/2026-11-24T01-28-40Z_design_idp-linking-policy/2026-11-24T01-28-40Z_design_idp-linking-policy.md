# IdP identity-linking policy — pending trust, passkeys, configurable factors, account merge

Date: 2026-11-24 · Source: Drew's policy statement, 2026-11-23 ~19:21 CST · Recorded on: [#13415](https://github.com/developing-today/code/issues/13415#issuecomment-7119068891) (policy), [#13415 gap analysis](https://github.com/developing-today/code/issues/13415#issuecomment-7119120391) · Workstreams: #13972 (HEAD), #13973, #13974, #13975

## The policy (Drew, verbatim in substance)

1. **Link, don't auto-trust.** All federation doors link to the same user, but a new provider sign-in does not automatically gain trust. Example: a user logged in with Google tries Microsoft — the Microsoft sign-in is allowed and registered, but grants no true access until the user, logged in with the already-established Google account, approves the link as the same account. After approval either provider signs in fully, and GitHub is added the same way.
2. **Auto-provisioning.** Whenever any new Microsoft, Google, or GitHub account is logged in, automatically create our own CA SSO user account for it.
3. **Passkeys.** Support passkeys. A passkey ties to the way it was signed in by default, but the user is presented an option to make it distinct — its own login, separate from SSO. Users may make as many passkeys as they want and attach them to one or more SSO providers.
4. **Configurable two-factor.** Users can enable two-factor as any combination they choose — SSO plus passkey, two different passkeys, or two SSOs — and configure it themselves.
5. **Account merge must be possible.** There can't be objects that are guaranteed unique per account (or only a limited set); on merge the user chooses which account is primary; no data is lost, everything joins; profile attributes (picture, summary) can be repointed one by one between the merged accounts.
6. **Undecided:** recovery requests and account deletion policy — Drew has not set a policy yet. Open question, decider Drew; not designed by any workstream below.

## What linking does today (evidence, 2026-11-23)

From `pkgs/idp-provider/src/identities.rs` (landed by PR #13782) and the live proof on #13415: the runtime identity store holds one row per `(provider, subject)` with status `active | unlinked` — the only two states. An allowlisted identity's first door login links it by that login (actor `door-login`, recorded in the append-only identity history and the audit record) and immediately mints tokens for the account; the config identity list seeds the store as `active`; later link changes are operator CLI acts (`idp-provider link|unlink|links`); an unlinked identity is denied at the door (`identity_unlinked`) while the account stays reachable via other doors; history rows are never hard-deleted. The trust anchor is the deny-by-default allowlist of Drew's exact upstream account IDs. The minimal provider has no authenticated session layer, no account-creation path, no passkeys, no factor concept, and no merge operation; the #9077 design note (`doc/2026-11-23_design_fleet_sso_9077.md` §4) had explicitly scoped MFA/passkeys out until this policy.

## The gaps in one paragraph each

**Pending trust.** No pending state, no claim mechanism, no approval act: first allowlisted login is full access for every provider alike. The allowlist makes that safe today; auto-provisioning removes the hand-curated list as the anchor, which makes the pending state load-bearing. **Provisioning.** No account is ever created by a sign-in; the single account row exists because config seeded it. **Passkeys.** None; the policy's binding shape (sign-in-path by default, standalone by option, unlimited, multi-provider attach) has no counterpart in the current model. **Two-factor.** None; one door login mints tokens, with no per-account authentication policy and no between-factors state. **Merge.** No operation and no uniqueness inventory; starting points are roles on the account row, identity rows keyed `(provider, subject)` → `account_id`, append-only history naming the original `account_id` forever, tokens carrying `sub` = account id, and the #2852 fold-path requirement that a merged account present exactly one binding target to the id system.

## The four workstreams (dependency order)

- **(a) #13972 — pending-link registration + approval from an established session. HEAD.** Adds the `pending` identity state, claim creation (session-add and email-match), auto-provisioning, and the minimal account session the rest build on. A behaviour change to the live-proven door path.
- **(b) #13973 — passkey support (WebAuthn).** Binding model (path-bound by default, standalone option), unlimited credentials, attach to one or more providers. Depends on (a)'s session.
- **(c) #13974 — configurable two-factor.** A per-account pair of distinct factor slots over the account's active providers and passkeys, self-configured, gating token issuance. Depends on (a) + (b).
- **(d) #13975 — account merge.** Uniqueness audit first (its rule binds (b)'s and (c)'s schemas), then the two-session merge operation, primary selection, no-loss join, per-attribute repointing. Independent of (a)–(c) except as noted in its issue.

## Open question carried, not answered

Recovery requests and account deletion policy — decider: Drew. Every workstream names it and designs around its absence.
