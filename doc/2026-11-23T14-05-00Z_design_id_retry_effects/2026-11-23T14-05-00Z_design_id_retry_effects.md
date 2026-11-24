# Recovering authority and effects across interrupted operations

**Design proposal dated November 23, 2026.** The motivating observations use the pinned sources below.

**Proposed adapter contract.** One user action may yield a durable intent, a remote acceptance, an external effect and one or more receipts. A renderer should expose the most specific fact the local node can prove. An absent reply cannot prove either failure or success. A separately authorized query can recover an earlier result without executing the effect again. [Existing MCP policy](https://github.com/developing-today/code/blob/7310a95730791e66da98f7b0e13865930058a516/pkgs/id/crates/id-caps/src/mcp_policy.rs) · [Relay design](https://github.com/developing-today/code/issues/2036)

| Recovery observation | Admissible action | Evidence exposed |
| --- | --- | --- |
| Content mutation locally durable, remote acceptance unknown | Resend the same signed identity under the current send grant | Local commit identity, remote state unknown |
| Authoritative effect already committed once and receipt readable | Retrieve the receipt under current read permission | Original command identity and observed effect status |
| Current grant revoked, prior receipt inaccessible | Refuse new effect; apply the explicit receipt visibility rule | Authorized refusal or unknown state without disclosing protected history |
| Effect provider is non-idempotent and outcome cannot be looked up | Keep the command uncertain and request an authorized reconciliation | Record that duplicate-safe recovery is unavailable |

**Compatibility.** Bind a request identity to principal, operation, arguments and effect identity. If the same identity arrives with different content, refuse. A transaction spanning local log and external service needs a provider idempotency or recovery contract. An outbox or a ledger of sent attempts alone does not establish exactly-once external effects. For a pure update replay, record deterministic inputs and the versioned guest profile. A host-sourced response enters replay only as explicit recorded input when policy permits disclosure.

**Worked acceptance case.** A model first accepts intent X, commits external result R once and loses the reply. The sender disconnects, restarts and attempts to resolve X after its grant changes. The independently specified external effect count is one. Current policy decides whether R may be read, whether a new send may occur and who may see the audit evidence. The two hosts agree on authorized observations at a common cut. Logs need distinct actors for queueing, custody and final acceptance. Apply this test first to a deterministic stub provider, then to one existing adapter with support for stable request identity. [MCP bridge follow-up](https://github.com/developing-today/code/issues/7917) · [Messaging implementation](https://github.com/developing-today/code/issues/5534)
