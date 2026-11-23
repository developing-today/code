# CLI-02 — Shared command contract and strict parser

Issue: [#9438](https://github.com/developing-today/code/issues/9438)
(slice `CLI-02` of [#9206](https://github.com/developing-today/code/issues/9206),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.** This
record elaborates the command-contract slice of the merged design
package (`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225) into the parser contract an implementation issue can be
scoped against. It does not re-decide anything that package settled,
and it does not authorize code, schema, migration, deployment, or live
acceptance; those remain separately gated stages (see §11).

Governing decisions consumed, not re-opened:

- **Architecture shared interface** (`architecture.md`, "Shared command
  and result interface" and "Digest and serialization contract"): the
  wire identifiers are `fleet.command/v1` and `fleet.result/v1`; the
  request-fingerprint DTO is `fleet.command-intent/v1`; framing is
  exactly `ASCII(domain) || 0x00 || UTF8(JCS(dto))` under RFC 8785,
  SHA-256 rendered `sha256:` + 64 lowercase hex; the digest-domain
  registry has exactly three purposes. This record specifies how an
  invocation becomes that envelope; it defines no second envelope.
- **CLI-01** ([#9451](https://github.com/developing-today/code/issues/9451),
  CLOSED): the shared Command/Result contract as landed in
  `pkgs/fleet/crates/fleet-queue/src/work/contracts.rs` — wire-version
  constants, the digest-domain registry, `digest()` /
  `compute_request_digest()`, canonical-decimal helpers, the
  reason-code registry, and the outcome set. CLI-02 consumes those
  types read-only; the parser validates against them and registers
  none of its own in that file.
- **#9206 §4**: parse the whole argv strictly before opening services
  or doing remote I/O; each command declares its exact accepted flags,
  aliases, values, and mutual exclusions; identity/output/safety flags
  are **not** globally accepted by implication. Shortcuts are
  compositions over the same envelope and service seam — no
  client-side policy, no independent resolver.
- **#9203 §3.1 + DSP-02**: the strict slash grammar (V2 proposed slash
  protocol, revision 1) folds into this slice (facade acceptance on
  #9438: `DSP-02` → `CLI-02`). There is one parser; no second slash
  parser may be built.
- **FDR-PMO-001** (canonical broker command seam): accepted as a local
  design contract, 2026-11-13. The parser's only output path is the
  typed envelope into that one seam. Global registration through
  #9180 remains separate and is not a blocker to this design work.
- **Operator-script argument contract (#3352)** (`AGENTS.md`;
  `doc/agents/operator-script-argument-contract.md`): the repo's
  standing strict-parsing rule for operator scripts. §4 consumes its
  strictness semantics for the Fleet CLI. One deliberate structural
  difference: #3352 parsing is per-tool with no shared library,
  because operator scripts are independent tools; the Fleet command
  surface is one product surface feeding one seam, so its parser is
  shared by design. The semantics — whole command line declared,
  strict parse before any effect, unaccounted input is a usage
  error — are adopted unchanged.
- **#9200 priority policy**: numeric priority *range and meaning* are
  policy-owned. The parser checks the shape of a priority argument;
  the policy validates its range (DSP-02's dependency note).

Verified state at writing (2026-11-23, origin/main `4cfceaa8a`):

- `pkgs/fleet/crates/fleet-queue/src/work/cli.rs` does **not** exist;
  neither acceptance test (`work_cli_rejects_unknown_args_before_side_effects`,
  `pmo_command_jcs_digest_conformance_cross_lang`) exists as code —
  both names occur only in the design package. No CLI-02 dated design
  record exists under `doc/`; this is the first.
- `contracts.rs` **does** exist and already carries the CLI-01
  contract, including the digest machinery (`digest()`,
  `compute_request_digest()`, `DIGEST_DOMAINS`). The missing piece
  this slice designs is the parser that produces validated,
  default-expanded input for those types.
- Both granular prerequisites named by the slice are **resolved**:
  CLI-01 #9451 CLOSED and DST-01
  ([#9501](https://github.com/developing-today/code/issues/9501), V2
  lifecycle state crosswalk) CLOSED. The formal `blocked_by` records
  added 2026-11-22 point at those two now-closed issues.
- The gated implementation follow-on already exists as queue task
  15953; §11 covers its refresh. No duplicate follow-on is created
  by this record.

## 1. Intent

Every Fleet command surface — an agent typing `fleet work …`, a
`/fleet …` comment on an issue, a PM Stack action request — must mean
exactly one thing, and must fail *before* it does anything when it
cannot be understood exactly. Today the surfaces parse independently
(or loosely), so the same intent can be spelled several ways, a typo'd
flag can fall through to a different operation, and a malformed
invocation can be discovered only after a service call has already
been made.

CLI-02 specifies the single strict parser that stands in front of the
FDR-PMO-001 seam: it turns an invocation into either (a) one typed,
validated `fleet.command/v1` envelope, or (b) a usage error naming the
offending argument, produced before any service is opened, any lock
taken, or any remote call attempted. Everything downstream — claim,
lifecycle transition, completion — consumes the envelope, never raw
argv or raw comment text.

## 2. One parser, three ingress shapes

The parser accepts three ingress shapes and normalizes all of them to
one closed typed AST, then to the shared envelope:

| Ingress | Raw form | Specified in |
| --- | --- | --- |
| CLI argv | `fleet work <operation> [flags]` token sequence | §3, §4 |
| Slash command | one `/fleet …` command in a comment body | §5 |
| PM action request | structured action via #9201, normalized to the same Command interface (#9203 §3.1) | this section |

Rules:

- **Closed AST.** The AST is one closed enum of operations with
  bounded, typed arguments (the DSP-02 round-trip/property oracle:
  every accepted input parses to exactly one enum variant; there is
  no "generic command with a bag of strings" variant).
- **One seam.** AST → envelope construction is the parser's last
  step. Policy, authorization, resolution, and execution all live
  behind the seam (#9206 §4: no client-side policy, no independent
  resolver). The parser never calls a service to decide whether an
  argument is *acceptable*; it only decides whether it is
  *well-formed against the declaration*.
- **Composed operations are spellings, not backends.** Sugar such as
  `fleet work next --claim --hydrate full` parses to the composed
  operation kind whose steps (selection → claim → hydration →
  journal/receipt) the service performs (#9206 §5). The parser does
  not expand sugar into multiple envelopes; composition is a service
  behavior, so a composed command still gets one `operation_id`, one
  digest, one receipt.
- **Registration glue is not CLI-02's.** Wiring the parser into
  `pkgs/fleet/crates/fleet/src/main.rs` and the `fleet-registry`
  surface is the single CLI-13 registration slice's ownership (slice
  boundary on #9438). CLI-02 owns the parser in
  `src/work/cli.rs`; CLI-13 owns its registration.

## 3. The argv grammar

### 3.1 Shape

```text
fleet work <operation> [<positional>] [--<flag> [<value>]]*
```

- `<operation>` is a single token from the operation inventory
  (full inventory: `census/commands.jsonl`; CLI-02's allocated rows
  are §3.3). An unrecognized operation token is a usage error, not a
  suggestion prompt.
- Positionals are declared per operation (count, type, order). A
  positional the operation does not declare is a stray positional —
  a usage error (§4).
- There is no global flag set. A flag token is legal only where the
  operation's declaration lists it.

### 3.2 Per-operation declaration

Each operation declares, in one place in `cli.rs`:

1. its positional parameters (name, type, required/optional);
2. every accepted flag: long name, any alias, value type
   (boolean / string / canonical decimal / enum / path), whether a
   value is required, whether the flag is repeatable;
3. its mutual exclusions and required-together groups;
4. which shared-flag families (§3.3) it participates in, flag by flag.

A flag family's presence in this section is a *vocabulary*, never an
acceptance rule: `--json` is accepted by an operation only because
that operation's declaration lists it (#9206 §4).

### 3.3 Shared flag families and CLI-02's allocation

The shared vocabulary (C-R108), grouped:

| Family | Flags |
| --- | --- |
| Identity / target | `--work`, `--issue`, `--pr`, `--repo`, `--agent-id`, `--session-id`, `--lease-token` |
| Time / cursor | `--since`, `--since-start`, `--since-checkpoint`, `--external-only` |
| Output | `--json`, `--format` |
| Safety / execution | `--dry-run`, `--explain`, `--expected-version`, `--idempotency-key`, `--verify`, `--wait` |

Value-shape rules the parser enforces (policy stays downstream):

- IDs, versions, generations, and lease generations are canonical
  decimal strings (CLI-01 helpers), never coerced through floating
  point; a value that is not canonical decimal where one is required
  is malformed, not rounded.
- `--lease-token` is accepted only as an opaque credential input; it
  is never echoed into output, logs, error details, or the intent DTO
  (#9206 §4).
- `--expected-version` and `--lease-generation` (carried by
  `--lease-token`'s companion state) are *separate* preconditions;
  the parser keeps them as distinct typed fields and never merges
  them into one "version" argument.
- A bare numeric target (e.g. `842` with no kind) is structurally
  ambiguous and is rejected by the parser; typed locators
  (`--work`, `--issue`, `--pr`, or a canonical `(authority_id,
  work_id)` pair) are the only target forms. Whether a well-formed
  locator *resolves* to exactly one item is the resolver's question
  (§10), not the parser's.

CLI-02's exact command allocation from `census/command-slice-map.jsonl`
(215 rows, no wildcard; full inventory remains the census):

- **Operations:** `agent.bootstrap`, `fleet.capabilities`,
  `fleet.errors`, `fleet.help`, `queue.search`,
  `github.comment.reply`, `github.plain-language-routing`.
- **Slash operations:** `agent.slash.status`, `agent.slash.triage`,
  `agent.slash.park`, `agent.slash.unblock`,
  `agent.slash.priority.high`, `agent.slash.priority.numeric`
  (§5; `start`/`resume`/`retry`/`do-now`/`merge`/`agent status`
  spellings map onto the same grammar per #9203 §3.1).
- **Flags:** `flag.assigned-to`, `flag.blocked`, `flag.body-file`,
  `flag.capability`, `flag.created-since`, `flag.estimated-effort`,
  `flag.exclude-label`, `flag.format`, `flag.json`, `flag.label`,
  `flag.limit`, `flag.queue`, `flag.queue-search.ready`,
  `flag.queue-search.type`, `flag.repo`, `flag.scenario`,
  `flag.status`, `flag.unassigned`, `flag.updated-since`.

## 4. Strictness rules

The parser's contract, in evaluation order. Rules 1–6 consume #3352;
rules 7–9 are the #9206 §4 additions for a service-backed CLI.

1. **Whole declaration, whole parse.** The complete token sequence
   is parsed against the operation's declaration before the parser
   returns anything. There is no streaming/incremental acceptance.
2. **Parse before effects.** No service is opened, no file is read
   for command input (beyond declared `--body-file`-style inputs the
   declaration itself names), no lock is taken, and no network call
   is made until the full parse and shape validation succeed. This
   is the property the first acceptance test names:
   `work_cli_rejects_unknown_args_before_side_effects`.
3. **One failure class for unaccounted input.** Unknown flag, stray
   positional, and typo'd flag are the same event: a usage error
   naming the offending argument. There is no "ignore unknown flags"
   mode and no prefix-matching of flag names.
4. **`--opt=value` ≡ `--opt value`, or refused.** The equals form is
   either accepted with identical meaning or refused outright for
   that flag in the declaration; it never falls through to a
   different operation or a default.
5. **Valueless means error.** A value-flag present without a value
   is a usage error, never an invitation to substitute the default.
   Defaults are applied only to *absent* flags, at default-expansion
   time (§6), where they become visible to the digest.
6. **Safety flags bind every path.** A parsed `--dry-run` /
   `--explain` is honoured on every code path the invocation can
   take, including error and partial paths; a safety flag the
   operation does not declare is unaccounted input (rule 3), not a
   no-op.
7. **Duplicates and conflicts are parse-time.** Repeating a
   non-repeatable flag, or supplying two flags from one
   mutual-exclusion group, fails the parse. Nothing is resolved by
   "last one wins."
8. **No inference at the parser.** The parser does not guess a repo
   from a bare number, an operation from a near-miss spelling, or a
   target from ambient state. Session/context defaults
   (architecture: "infer active context only when unambiguous") are
   applied by the service behind the seam from recorded session
   state, and the *expanded* values — never the inference process —
   are what enter the envelope and digest.
9. **Unknown schema versions refuse.** An envelope or DTO whose
   `schema_version` is not one the parser/contract recognizes is
   refused with the CLI-01 contract error, never best-effort parsed
   (CLI-01 owns the version behavior; the parser surfaces it).

## 5. Slash grammar (DSP-02, folded into CLI-02)

The slash surface is the same parser fed from comment text. Grammar
(#9203 §3.1, V2 proposed slash protocol revision 1 — a proposal to
implement, not a claim about currently supported commands):

```text
/fleet status
/fleet triage [now]
/fleet start
/fleet resume
/fleet retry
/fleet park [--reason <quoted text>]
/fleet unblock
/fleet priority high | /fleet priority <integer>
/fleet do-now
/fleet merge
/fleet agent status
```

Rules:

- **Closed verb set.** The verb after `/fleet` is one of the grammar
  above. Unknown verb, arbitrary flags, unrecognized trailing text,
  `;`, shell expansions, and URLs-as-payload all fail the parse.
  One command per comment; a second command is unaccounted input
  until a separately reviewed multi-command model exists.
- **Mapping is total and explicit.** Every accepted slash form maps
  to exactly one operation kind in the shared envelope (e.g.
  `park` → `work.park` with the reason carried as the envelope's
  typed reason, `priority <integer>` → the priority operation with a
  shape-valid integer the #9200 policy then range-checks). The
  mapping table is part of the parser's fixture corpus (§8).
- **Aliases are candidates, not behavior.** Informal spellings found
  in the sources (`/retry`, `/queue`, `/start`, `/merge`) are alias
  *candidates* only. An alias is accepted only after an explicit
  mapping to one canonical verb is approved, documented, and tested;
  ambiguous forms are rejected. No alias ever resolves to a shell or
  runner invocation.
- **Syntax is not consent.** `/fleet merge` parses like any other
  verb; merge authority, clean preview, checks/reviews/head-SHA
  readback, and explicit confirmation are service-side preconditions
  (REL-06's territory), and a parse success asserts none of them.
  Likewise `start`/`resume` on blocked or parked work invoke the
  hard gates behind the seam; `retry` is a typed retry request, not
  an attempt-counter reset; `park` requires a reason under #9179's
  terms, carried structurally, not as a free-form status write.
- **Plain language is classification, never execution.**
  `github.plain-language-routing` classifies free text into
  candidate intents with confidence and explanation; the output is
  untrusted data for triage, separately authorized by actor policy.
  Low-confidence or ambiguous text is captured for triage, not acted
  on. A classifier cannot decide that an author has permission
  (#9203 §3.1).
- **`do-now` is its own verb.** It is not the `expedite=15` /
  `next-up=25` priority bands: those ratified spellings and values
  are unchanged by this grammar, and the parser must not normalize
  one into the other (§9).

## 6. From AST to envelope: default expansion and the intent digest

Once parsing succeeds, the parser (with the contract types) produces
the envelope and its fingerprint in this order:

1. **Default expansion.** Declared defaults are applied to absent
   flags. Expansion is deterministic and total: the expanded
   argument set is the only form that crosses the seam. No wall-clock
   or freshly fetched state may be injected as an implicit default
   in a way that changes a retry's fingerprint (architecture digest
   contract).
2. **Intent DTO construction.** The DTO has exactly the architecture's
   keys — `schema_version` (`fleet.command-intent/v1`),
   `command_schema_version` (`fleet.command/v1`), `work`, `kind`,
   `arguments`, `expected_work_version`, `lease_generation`,
   `observed_snapshot_ref`, `owner_policy_ref`, `reason` — nullable
   fields present as `null`. `owner_policy_ref` is a meaningful
   intent precondition: changing it changes the digest.
3. **Digest.** Computed with CLI-01's `digest()` under domain
   `fleet.command-intent/v1` only. The parser never selects a domain
   from input, and never touches the `fleet.context-manifest/v1` or
   `fleet.selection-order/v1` purposes or the bounded
   creation-outbox hash.
4. **Pre-hash refusals.** Duplicate properties, invalid Unicode,
   NaN/Infinity, and lossy numeric IDs are rejected before hashing
   (architecture). JCS performs no Unicode normalization and no set
   reordering; any semantically unordered argument set is normalized
   by the command schema (i.e. at declaration time in the parser's
   per-operation argument construction), not during canonicalization.
5. **Digest ≠ authorization.** Idempotency lookup is scoped by
   authenticated principal + authority, and authorization is
   re-checked at acceptance; a matching digest recovers the original
   operation, and never grants anything. Excluded from the DTO, by
   the architecture and restated here because the parser owns the
   boundary: operation/key IDs, delivery attempts, receipt times,
   output format (`--json`/`--format`), wait/timeout, and dry-run
   execution mode. `--dry-run` still changes *behavior* (no effects);
   it does not change *intent identity*.

## 7. Parser errors and the result surface

- **A parse failure creates nothing.** No `operation_id` is minted,
  no envelope is submitted, no receipt exists. The failure is
  reported as an invocation error: a stable error code, the
  offending argument, and the operation's usage shape. This is the
  one output of the CLI that is not a `fleet.result/v1` envelope,
  because no operation ever existed.
- **Error codes are stable strings, registered once.** The parser's
  codes live with the parser in `cli.rs` (CLI-02 does not own
  `contracts.rs` and does not extend CLI-01's `ContractError`).
  Candidate set the implementation issue starts from:
  `unknown_argument`, `unknown_operation`, `missing_value`,
  `duplicate_flag`, `conflicting_flags`, `malformed_value`,
  `ambiguous_target_form`, `unrecognized_trailing_input`. Renaming a
  shipped code is a breaking change; adding one is not.
- **Exit status.** A usage error exits non-zero — 2, per #3352's
  default where the tool has no established usage code. A successful
  parse that reaches the seam reports its outcome *in the envelope*
  (outcomes: `rejected`, `accepted`, `running`, `pending_sync`,
  `partial`, `conflict`, `uncertain`, `verified_complete`, per
  CLI-01/architecture); exit status signals the invocation/domain
  result only and never substitutes for the durable receipt. The
  full exit-code ↔ outcome conformance matrix is an implementation-
  issue deliverable (#9206 §4 leaves the exact mapping to that
  matrix); this record fixes only the parse-failure row of it.
- **Human and JSON output render the same facts.** `--json` changes
  rendering, not content: the same envelope, the same error object
  (`code`, `retryability`, safe message, current version/resource
  where relevant, `operation_id`, remediation). No secret material —
  lease tokens above all — appears in either rendering (§3.3).

## 8. Acceptance oracles, specified

The implementation issue inherits these exact tests from #9438; this
record specifies what each must contain to be a real oracle.

### 8.1 `work_cli_rejects_unknown_args_before_side_effects`

- The test drives the parser against an effect-recording seam: a
  test double behind the service boundary that counts every service
  open, lock, file, and remote call. For every negative case, the
  recorded count must be **zero** and the result must be the usage
  error naming the offending argument.
- Negative corpus, minimum: unknown flag; typo'd flag; stray
  positional; missing flag value; `--opt=value` on a flag declared
  space-form-only (or the reverse); duplicate non-repeatable flag;
  two flags from one exclusion group; undeclared safety flag;
  bare-numeric target; unknown operation token; trailing tokens
  after a complete invocation.
- Positive control: the same operations, well-formed, parse and
  reach the seam exactly once — proving the harness would have
  recorded an effect had one occurred (a zero count from a dead
  harness is not evidence).

### 8.2 `pmo_command_jcs_digest_conformance_cross_lang`

- Consumes the golden vectors in the design package's
  `contracts/digest-vectors.json` (byte framing + DTO fixtures). The
  Rust side computes digests through CLI-01's `digest()` from
  parser-produced, default-expanded DTOs; the TypeScript side
  computes independently; canonical bytes and `sha256:` outputs must
  agree exactly, per vector.
- Cases must include: fixed default/null semantics (absent vs
  explicit `null` vs defaulted values digest differently only where
  the DTO actually differs); the excluded-fields rule (varying
  output format, wait budget, or dry-run mode over one intent yields
  one digest); the `owner_policy_ref` rule (changing only it changes
  the digest); and the pre-hash refusals of §6 rule 4 as negative
  vectors.
- Ordinary language-default JSON sorting is not JCS and is not an
  acceptable substitute implementation of the comparison.

### 8.3 Slash corpus (DSP-02's oracle, discharged here)

DSP-02 named its failing test `pmo_dsp_02_command_grammar_is_closed_and_typed`
and placed it in `contracts.rs` contract tests. One deliberate
placement correction, recorded here: CLI-02's owned-file list makes
`cli.rs` the parser's home and `contracts.rs` a read-only consumer,
so the grammar test lives with the parser in `cli.rs`'s test target
under the same name. The corpus itself is unchanged: accept every
canonical §5 form; reject unknown command, executable syntax,
duplicate command, arbitrary option, malformed score, ambiguous
target, unsupported alias, and a merge form lacking its confirmation
structure; plus a round-trip/property check that every accepted input
yields one closed-enum verb with bounded typed arguments. If a PM
ingress adapter keeps its own parser, it runs the same fixture
corpus (`pm-stack/dispatch` bun tests, per DSP-02) — against these
fixtures, not a fork of them.

### 8.4 Scenario command lines

The reference scenarios (C-R149/C-R150/C-R152) are parser coverage:
every command line in Scenario A (`fleet work next --ready --claim
--hydrate full`, `fleet work note add`,
`fleet work changes --since-start --external-only`,
`fleet work complete --verify --wait`, …) and the slash forms in
Scenario C must appear in the golden corpus as accepted, with their
expanded DTOs pinned. A scenario spelling the parser rejects is a
design contradiction to surface, not a test to delete.

## 9. Compatibility and migration

- Existing queue/status/identity/Project-writer and Fleet `gh`
  compatibility surfaces are preserved. This design changes no
  existing spelling and removes none.
- `expedite=15` / `next-up=25` are ratified priority-band spellings
  and remain distinct from the `do-now` verb (§5). The parser
  normalizes neither into the other.
- The `--merge-linked-prs` / `--close-linked-issues` family vs the
  explicit `--merge-pr` / `--close-issues` family question (#9206
  §5) is **not** decided here: both families stay in the declaration
  vocabulary, neither is silently dropped, and the alias-or-canonical
  decision is recorded as an open item for the implementation issue
  to raise explicitly rather than settle by omission.
- QTI remains unlocated/unverified (#9241 owns provenance recovery);
  no persisted field or schema is touched by this design, so no QTI
  crosswalk gate is triggered.
- Rollback for the future implementation is disable-only: the new
  parser is bypassed, pending intents and existing state are
  preserved, no history is deleted (slice compatibility terms).

## 10. Boundaries with sibling slices

- **CLI-03** ([#9439](https://github.com/developing-today/code/issues/9439),
  OPEN): read-only work inspect/status and identity resolution; owns
  `src/work/lifecycle.rs`. The overlap is real but the boundary is
  clean: **CLI-02 owns how an invocation is spelled, declared, and
  strictly validated; CLI-03 owns what `work.inspect`/`work.status`
  return and how a well-formed locator resolves to a work item.**
  The parser hands CLI-03 typed locators and flags; resolution
  outcomes — including `AMBIGUOUS_RESOURCE` when a locator matches
  more than one item — are produced behind the seam by the resolver,
  never by the parser, and CLI-02 designs no resolution semantics.
  Shared allocated IDs (C-R005, C-R009, C-R107, C-R108, …) are
  answered by each slice for its own half.
- **CLI-01** (#9451, CLOSED): owns `contracts.rs`. CLI-02 consumes
  it read-only (§8.3 records the one placement correction this
  forces for the DSP-02 test).
- **CLI-13**: owns CLI registration glue (§2). CLI-02 delivers the
  parser; it does not register it.
- **#9203's other DSP slices**: DSP-01 (webhook intake), DSP-03
  (idempotency ledger), DSP-04 (router/policy), DSP-05–DSP-10 keep
  their own ownership. Only DSP-02's grammar folds here; its routing,
  authorization, and delivery semantics do not.
- **REL-06** ([#9516](https://github.com/developing-today/code/issues/9516)):
  owns effect preconditions and read-back verification behind the
  seam. The parser guarantees an invocation is well-formed; REL-06
  guarantees an effect was safe to send and is verified after.

## 11. Gates and sequencing

Implementation of this design starts only when **all** of the
following hold (slice runtime gates, restated):

1. This record is merged.
2. The granular prerequisites are resolved — verified at writing:
   CLI-01 #9451 CLOSED, DST-01 #9501 CLOSED, PR #9225 MERGED. The
   FDR-PMO-001 namespace check is discharged by that record's
   accepted local status; #9180 global registration is pending and,
   per the slice body, is not a blocker.
3. A separately scoped implementation issue is filed against this
   record and the §8 oracles. The existing gated follow-on, queue
   task 15953, is that vehicle once its packet is refreshed against
   the current crate layout (its 2026-11-16 gate note flags the
   `pkgs/fleet/Cargo.toml` feature set — `client,queue-broker`, as
   the issue's done command states — for re-verification at filing
   time). #9438 itself authorizes no code.

Schema/migration, production rollout, authenticated remote writes,
and live acceptance each require their own explicit evidence and
approval. This design creates, changes, or retires **no schedule on
any surface**; `doc/fleet-schedule-registry.md` is intentionally
untouched.

## 12. Requirement trace (design-level)

Full per-occurrence text for every allocated ID is in
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl`;
this table records only which part of this design speaks to each
group — it claims design coverage, not implementation.

| IDs | Where addressed |
| --- | --- |
| C-R005, C-R009, C-R107, H-R017, O-R023 | §2, §3 — one cohesive command surface across the agent/queue/work/GitHub/dependency/metadata/sync/session families, names reconciled to Fleet conventions |
| C-R108, O-R024, P-R005, P-R009, P-R012 | §3.3, §6 — the shared identity/time/output/safety flag vocabulary, per-operation declaration, default expansion, unambiguous context only |
| C-R086, C-R087, O-R014, S-R004 | §5 — the strict slash grammar, its closed verb set, alias discipline, and plain-language classification boundary |
| C-R106, C-R109, C-R110, P-R003 | §2, §3 — composed operations and sugar over the one envelope/seam; primitives and workflows share the grammar |
| C-R149, C-R150, C-R152, C-R156 | §8.4 — reference-scenario command lines pinned as golden parser coverage |
| P-R024, P-R028 | §6, §7 — stable JSON output, stable error identifiers, distinct outcome states, auditable normal path and recovery via `operation_id` |
| H-R012, H-R016, H-R024, H-R031 | §8, §9 — original commands/options/schemas/errors/failures carried into compatibility and acceptance coverage; nothing silently dropped |

## 13. Non-goals

- No implementation, and no new queue, database, scheduler, service,
  resolver, executable, GitHub client, or general creation-outbox
  producer (slice boundary). No edits to `contracts.rs`, `client.rs`,
  `lib.rs`, `main.rs`, or the registry — in the future implementation
  exactly as in this record.
- No change to #6533 or its live acceptance; dispatch routing stays
  #9203's.
- No full exit-code ↔ outcome matrix (§7 fixes only the parse-failure
  row; the matrix is the implementation issue's conformance
  deliverable, per #9206 §4).
- No alias approvals: §5 admits alias *candidates* only; approving
  any one of them is a separate, documented decision.
- No identity-resolution semantics (§10: CLI-03's), no merge/effect
  preconditions (REL-06's), and no priority ranges (#9200's policy).
