# Dedicated tracker backend

The API consumes a D1-shaped database adapter. The private production topology uses Turso; local fixtures and the separately hosted Sites prototype use D1-shaped storage. The API does not use Atlas or Durable Objects. Optimistic versioning and atomic batches protect edits; concurrent changes return 409 for explicit reload and reconciliation.

An authenticated hosting gateway supplies trusted identity headers. The Turso Worker strips caller-provided identity, verifying the existing signed Tailscale gateway instead. Direct public Worker deployment without identity-header protection is unsupported. Writes require same-origin JSON requests. Every store read checks membership and private/shared scope. Shared does not mean public: it means invited members. Roles are reader, editor, admin; the creator retains admin and alone changes store type. Archived stores remain readable but reject content edits.

No invitation emails are sent. An email-bound pending invitation appears when that account accesses bootstrap; the account must first be allowed through the Site platform's access policy. Creating a store invitation does not change Site sharing. Do not send or grant access without the owner's authorization.

Document events are append-only and store authoritative before-state, authenticated actor, server time and resulting state. Client-supplied before/actor/time are not trusted. Command IDs are scoped to the actor, store and canonical command intent. Store and invitation/role changes have a separate access audit. Database schema never changes in request handlers.

Nickname, archive visibility, hidden visibility and per-store user settings are separate from shared content. Snapshot state may still contain legacy browser preferences; frontend should use per-user settings.

Limitations: snapshot per event is intentionally simple and bounded to 1.5 MB. History reads are not paginated yet; very large histories need pagination/checkpoints before large-scale use. No arbitrary remote database credentials or replication endpoints are accepted. Real-time push and automated AI extraction are not implemented.

Local checks:

- node --check worker/api.mjs
- node tests/backend.mjs
  Tests use native Node SQLite through a D1-shaped adapter. Hosted D1 + gateway must still be verified after deployment.

## V2 contract and limits

POST /api/provision {initialState} creates an empty personal store per authenticated account and one Site default shared store seeded by its first creator, idempotently. The display name is a nickname or Personal; identity is server-derived, never a browser UUID. Subsequent callers do not join the default shared store. Bootstrap includes personalStoreId and defaultSharedStoreId only if accessible. Default audience types are locked; names remain editable.

Open is an application audience, not public site access. Open stores start empty with their own categories. Signed-in Site visitors can read current visible items/tags, but unjoined readers get no notes, inventory, research, policy state or history. Only members with editor/admin may write. Conversion into/out of open is blocked to avoid exposing historical data. Archived open stores are inaccessible to nonmembers.

GET /api/search?q=... searches accessible current item names/descriptions/attributes, excludes archived stores/items, hidden items, and stores hidden in settings.stores[storeId].hidden. It returns at most 100 hits with a truncated marker. Hide is presentation state, not access revocation. showHiddenStores is a user setting. No Atlas, Snowflake or new sign-in provider is connected.

Migration 0001 adds constant-default is_open instead of rebuilding the existing store table; shared plus is_open is represented as open in the API. Existing rows remain unchanged.

## Granular access compatibility

See [identity and access](../design/identity-access.md) for the live-policy contract and projection semantics. This release intentionally suppresses snapshot history in scoped views and blocks their generic snapshot editing rather than risk disclosing or dropping restricted rows. A future normalized event implementation is needed before enabling unrestricted editing and historical replay for partially authorized users.

Anonymous-first drafts and secure account linking require separate authenticated session and proof validation. A shared browser UUID, nickname or typed email is never evidence of account control.
