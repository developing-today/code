# Implementation staging and evidence gates

**Status:** proposed sequencing; numeric issue receipts and the final dependency
index are added after publication. Proposal identifiers below are not GitHub
issue numbers. This plan does not mark their implementation complete.

## First independently executable stage

Start with **TRACE-02** (the existing CLI/profile compatibility corpus) and
**PERF-01** (a reproducible fake-provider benchmark harness). They operate
against fixtures and the current command contract, require no live vault,
signer, new service, trust or DNS, and do not depend on a replacement backend.
The required **PERF-05** public-source SecureStore evaluation may proceed in
parallel; adopting it is conditional, doing the evaluation is not.

Expected stage output is checked-in test inputs/oracles, reproducible commands
and redacted evidence—not a production benchmark or proof that a new provider
is faster. TRACE-02 distinguishes intentional secret stdout from prohibited
logging. PERF-01 preserves errors/timeouts and labels preliminary quantiles
and assumptions; a synthetic harness timing is not hosted-provider latency.

## Additive implementation frontiers

| Frontier | Deliverables / enabling outcome | Exit evidence | Not authorized or required |
|---|---|---|---|
| F0 — baseline and contracts | TRACE-01/02, PERF-01/05; source/trace regression guard and compatible existing behavior | Exact source witnesses; negative fixtures for unmapped requirements; CLI/stdout/stderr/exit/capability goldens; deterministic benchmark statistics tests | No live provider inventory, no latency claims from anecdotes |
| F1 — reusable seams | ARCH-01/06/07/10, SEC-02/03; stable logical IDs, typed values, authority/capability boundaries, registry and dependency-closure contracts | In-memory adapters compile; secret redaction/type constraints; tiny-profile dependency/size checks; grant replay/concurrency/crash tests | No crate-per-noun explosion, competing CLI or CA daemon; no ambient root discovery |
| F2 — independent feature slices | Scoped BWS/OP adapters, PERF-02/03/04/06, GEN work and native formats, ROOT/CERT model/backend slices when their actual hard prerequisites are satisfied | Provider conformance/fidelity and fake-clock freshness tests; SOPS recipient/store-closure proof; unbiased generator properties; algorithm/encoding vectors and independent verifiers | No assumed provider CAS; no mandatory fnox adoption, SecureStore format migration, OpenBao or MTC |
| F3 — administration and lifecycle | REC inventory/planner/guards/journal/history/apply; root/issuer/leaf lifecycle, CRLs, publication plans and artifacts | Fault-injected sagas; 99 successful items checkpoint while one conflict remains unresolved; unchanged native IDs; exact graph, serial and authorization receipts; original-CDP update tests | No distributed-ACID claim, no deletion from partial scope, no immutable initial-empty-CRL trap; no production writes |
| F4 — integrated synthetic acceptance | OPS command/help/schema/man/completion/skills evidence and full Fleet-generated reference package | Use the finished Fleet binary itself; independent signature/path/client/format checks; PASS/FAIL/UNSUPPORTED by pinned version; restore and rollback rehearsals | A graph model, parser success, producer self-verification or mock alone is not production readiness |
| F5 — separately approved operations | Exact production identities/scopes, first reviewed inventory/reconciliation, custody/signing/publication/trust/DNS/activation and operational scheduling | Named grants, bounded plans, recovery evidence, accountable owner and staged rollout/rollback receipts; schedule registry updated with actual schedule changes | This design mission grants none of these operations; production children remain blocked until their own gates pass |

These frontiers are not blanket blocking edges or a waterfall. The **specific
issue dependency graph** determines readiness. PKI and generator library work
can advance independently of hosted-provider approval. An optional research
experiment may fail its go/no-go without blocking the core graph or secrets
engine. A design-layer label (V2/V3/V4) is a horizon, not an automatic runtime
dependency on every item in an earlier layer.

## Critical ordering decisions

1. **Oracle before compatibility refactor:** TRACE-02 owns existing behavior
   fixtures; ARCH-01 consumes them rather than duplicating a baseline.
2. **Descriptors before generated docs:** ARCH-07 establishes the command and
   capability descriptor seam; OPS-05 consumes it. Coordination in the reverse
   direction is soft, not a dependency cycle.
3. **Interface before adapter:** ARCH-06 owns generic signer/artifact authority;
   ROOT/CERT and generator code consume it. Key generation, root identity and
   cert variants are separate from the primitive interface.
4. **Final bytes before signing grant:** bounded ceremony/storage approval →
   subject SPKI available → durable serial reservation → exact TBS/tuple plan
   frozen → signature authorization → atomic single-winner consumption →
   signing → verified receipt. Dry-run placeholders are not final grants.
5. **History before destructive overwrite:** journal intent and required
   recoverable history/readback precede an in-place current-item update.
   Missing provider CAS defaults to refusal; a named weaker operating policy
   cannot claim to preserve an unseen concurrent edit.
6. **Per-item progress before global success:** commit a fully verified item's
   common baseline without reapplying it after restart. Unresolved items keep
   their old base; scope/global incremental watermarks advance only under
   their independent completeness conditions.
7. **Identity before revocation/publication:** the frozen exact signer graph
   determines issuer-key/serial and CRL ownership. Numbered CRLs are immutable
   archives; the original embedded current-CDP endpoint must serve subsequent
   verified higher-numbered CRLs for the same key/generation.
8. **Escrow policy before adapter; rehearsal after adapter:** the closed
   three-destination policy and independent recovery dependency design are
   prerequisites to escrow code. Restore rehearsal consumes the adapter;
   neither may hard-depend on its own downstream rehearsal.
9. **Verified issue before queue coverage claim:** actual numeric issue,
   native parent/dependency and independently observed live task receipts
   must exist. A dry run, search miss, enqueue acknowledgment or local intent
   is not all of those records.

## Parallelism and shared-file ownership

Each implementation issue names its owned modules/interfaces. Use independent
worktrees and a single writer per file. Pure parsers, fake-provider tests,
format adapters and research reports are good parallel slices. Shared registry,
`Cargo.toml`, public schema/API and central index edits are coordinated through
their designated owner; two workers do not independently expand the same
public interface. If a concrete shared-file collision cannot be removed by a
seam, record a sequencing edge before dispatch rather than discovering it at
merge time. Do not add unrelated PM-stack work to this CA hierarchy.

## Approval and rollout are separate from code completion

Exact account/project/vault/operator identities belong to a later approved
operation. They do not block public-source research, mock implementations or
synthetic tests. Conversely, compiling an adapter or satisfying mock tests
does not authorize a real inventory, root export or credential rotation.

The implementation epic and its domain/implementation children stay open
after this design is delivered. Closure requires each issue's actual
acceptance evidence, not the presence of a design packet or a queue task.
Nonblocking research retains a horizon, hypothesis, source references,
go/no-go and owner; it is not silently discarded if current infrastructure
cannot run its experiment.
