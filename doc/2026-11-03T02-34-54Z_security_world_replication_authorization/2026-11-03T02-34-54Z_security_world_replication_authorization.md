# World replication authorization

## Request and intent

Issue #4723 reports that the world-replication ALPN accepts any peer with a nonzero nonce and streams the raw journal. Require authorization before the journal is opened for replication, so an invalid or missing credential cannot receive journal frames.

## Design

Require the existing hub admin credential in `ReplicationRequest`, bound to the repository's admin-token limit, and validate it with `WorldHub::admin_ok` before resolving the world or opening a replication feed. Participant capabilities authorize the projected participant view, but this ALPN sends raw journal entries, including private chat and program seeds; they are not sufficient authority for this feed. The existing process-wide admin credential is already the authority for this unprojected host data, so no parallel auth mechanism is introduced. Keep the exact journal replication format unchanged. Do not log or derive `Debug` over the request's credential.

Invalid or missing credentials therefore neither resolve/open a world nor receive journal bytes.
