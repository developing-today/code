# Optional persisted-directory routing guard (#3399)

## Intent and boundary

Add a read-only check before the hosted front door can touch the USERS
namespace. When `HOSTED_DIRECTORY` is configured, a validated persisted
account with the requested binding ID and jurisdiction must exist. Missing
rows (including legacy-only inventory), invalid or migration metadata,
jurisdiction mismatch and database failures refuse routing. Reads never
signup, inventory, migrate or repair accounts.

`HOSTED_DIRECTORY` accepts the structural `HostedDirectoryDatabase` adapter,
not a raw Cloudflare D1 binding: statement `all()` returns rows directly;
D1 wraps those rows in `results` and requires an adapter. No production
Wrangler binding, deployment or provisioning is added. With no adapter the
existing local scaffold still consumes upstream context headers; neither
mode authenticates those headers or authorizes production enablement.

The lookup exposes the existing stable-record parser and validates binding
input before querying. Database failures become a generic 503 rather than
falling back to the scaffold. The directory read is a snapshot, not migration
CAS or a routing lease: #3404 owns migration coordination separately.

## Verified when

The actual `worker.fetch` entry point, using SQLite with the shipped
migrations through the directory adapter, forwards a matching stable row
and makes zero namespace calls for every refused case. A before/after
snapshot proves the requests do not mutate either directory table. Existing
no-directory front-door tests retain local scaffold behavior. Targeted
front-door and persistent-directory tests plus TypeScript checking run on
Fleet remote runners, with failing-first evidence before implementation.

## References

- [Directory implementation](../../hosted-worker/src/persistent-account-directory.ts)
- [Worker entry point](../../hosted-worker/src/worker.ts)
- [Front-door tests](../../hosted-worker/test/front-door.test.ts)
- Issues #3399 (slice), #3404 (migration CAS), #2285 / #3403 (enablement)
