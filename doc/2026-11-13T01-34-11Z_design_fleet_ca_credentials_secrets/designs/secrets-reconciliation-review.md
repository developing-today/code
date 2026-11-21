# Independent review receipt — secrets and reconciliation

**Reviewer:** read-only workhorse session `ses_e5e2d346d27ffeifGwyr0MCCAm`  
**Review result:** Agree; no remaining concrete blockers.  
**Scope:** latest versions of `designs/secrets.md`, `designs/reconciliation.md`, `research/secrets-prior-art.md`, and `proposals/secrets-reconciliation.json`; cross-checked against architecture ADR-S01/S02/S05, OPS-10, and ROOT-09/ROOT-10.  
**Evidence status:** these are design requirements/model oracles and code-source citations, not claims of provider behavior, implementation, deployment, grants, or live reconciliation.

## Reviewed-file SHA-256

Hashes are for the four reviewed deliverables above; this receipt is a separate file.

| File | SHA-256 |
|---|---|
| `designs/secrets.md` | `584f79c57ae0e9d495df491a8de5087195ffdf670f24a34c664ba58b2ea62819` |
| `designs/reconciliation.md` | `78ac4cb422ec40cf48603488a7e1f56fc1a00ffa858fb4a9d70df4c065a21d63` |
| `research/secrets-prior-art.md` | `6d350580a0362ed98b905001434b6634adda948f5c4cd8f522881abab15c2178` |
| `proposals/secrets-reconciliation.json` | `f09135b14e0affb106f0076f9f0c67813321b90eff59d90f83dc8166cedd25ec` |

## Quoted findings

- **Legacy root activation gate** — `designs/secrets.md`: “Before the first root escrow copy, the trusted root policy must bind the exact escrow project/container; root native IDs are registered from each escrow write receipt before any ordinary principal can gain scope.” Reviewer agrees OPS-10/ROOT-09 own the activation boundary; no grant/project changes are authorized here.
- **Trusted classification** — `designs/secrets.md`: “`DataClass` must come from trusted policy bound to provider/account/container/native IDs, never a caller-supplied label or mutable provider sidecar.”
- **Ambiguous absence** — `designs/reconciliation.md`: “Two complete-looking scans plus the same token hash do not prove deletion.” The same-principal ACL-shrink + two-list + 404 case remains `MissingUnverifiable`; deletion evidence or reviewed operator confirmation is required.
- **Independent baseline components** — `designs/reconciliation.md`: “A fully read, successfully reconciled item may atomically advance its own verified component even if a different item conflicts, is inaccessible, or a scope inventory is incomplete.” The global watermark remains unchanged while scope is partial or items are unresolved.
- **Outbox separation** — `designs/reconciliation.md`: “Outbox lag does not undo or block a locally durable, provider-readback-verified per-item baseline component unless an explicit deployment policy names the remote sink as a required durability dependency.”
- **Restart oracle** — `designs/reconciliation.md`: “Preserve 99 committed item components and do not repeat their history/provider writes on restart.” The conflicted item retains its old base and the global watermark stays unchanged.
- **Provider history limits** — `designs/reconciliation.md`: “Do not claim a hosted provider's copy is WORM/immutable unless that exact version/capability is proven.” Tamper/disappearance triggers integrity incident and reviewed additive restoration from an authenticated protected snapshot/receipt.
- **Journal rollback limit** — `designs/reconciliation.md`: “A chain alone cannot detect suffix rollback/truncation if an attacker can replace the journal and its local head with an older valid prefix.” Tail-rollback detection requires an independent monotonic head anchor; compromise of both anchor domains is outside the stated guarantee.

The reviewer found no remaining concrete blocker across the reviewed files. The 100-item model oracle is sound: 99 verified components remain committed, the conflicted item retains its prior base, and the global watermark remains unchanged.
