# Design: Decisions system (P2 design family)

- Date: 2026-11-24 (UTC)
- Source issue: developing-today/code#12951 (epic: decisions system — P2 design family, tracking home)
- Parent hub: developing-today/code#8986 (design register). Umbrella #13871, tracking #13873.
- Queue task: 21540 (repo-health-code-pilot, feature_slice, slice 1)
- Status: draft for ruling. Design only; no implementation in this document.
- Register reference: `pm-stack/design-register-2026-11-11.md`, §P2.1–P2.3.

## 1. Problem and context

The register lists **P2 Decisions system** as a parent design with three children. Only one child has a
tracking issue, and the family has no design of its own. The gap was flagged by the 2026-11-23
reconciliation (gap 2) and by AUDIT-7 Lane 6.

What exists today, with evidence from the register and the repo:

- **P2.1 Pending-questions ledger.** Built and running. Ledger state at the 2026-11-11 records pass:
  150 items (answered 84, asked 34, pending 20, dropped 12). Tracked by #8831. It has no single GitHub
  issue; the ledger file is the artifact.
- **P2.2 Decisions page — batched decision sets.** Shipped. v1 PR #8767, v2 PR #8770, drafts in PR
  #8778 and PR #8784. Tracked by #8766 (closed). A crash follow-up, #8777, is still open: the
  aggregator and dispatch units were SIGTERM'd and did not self-heal. Who sent the signal is not
  established.
- **P2.3 Decisions ledger — pending design rulings.** Not a system. It is a list of rulings still
  waiting on Drew (for example fleet tiers #2230, lifecycle verdict #2311, replica operator #806,
  design adoption opencode#1678, watcher scope openchamber#1441, daemon shutdown semantics #1414,
  Luna route #8735). Most have no tracking issue; the register is their only record.
- **Decision records.** Fleet rulings about queue semantics and supervision are meant to live as FDR
  records in `doc/decisions/` (#9180). A ruling that exists only in chat or a queue row is unrecorded.
- **Decision models.** The pool/effort brief supplement (2026-11-13) asks for a decision-model
  system: System 1 decisions (JSON state and typed questions in, probabilities out), decision pools
  with a default, and a cost-aware choice between native and LLM decision models.

What breaks today:

1. Rulings are scattered. A decision can be asked, answered, and lost, because nothing links the
   question to the record that governs the behaviour.
2. P2.2 has no crash-safety design. Its follow-up is an open bug without a design boundary.
3. P2.3 has no owner-readable list with states and deciders. Items sit in a table that is not
   maintained as a living ledger.
4. The decision-model pools have no design, so the cost and routing rules in the brief are untested.

Evidence: the links in the source issue, #13873, #13871, and the register sections cited above.

## 2. Goals and non-goals

Goals:

- G1. One design family home: this document, linked from #12951, indexing P2.1, P2.2 and P2.3.
- G2. A single ledger of open rulings with state, decider and link. It is the source for the decisions
  page and for the hourly questions job.
- G3. Every ruling that governs fleet behaviour is recorded as an FDR, not only in chat or a queue row.
- G4. A design for the decision-model pools (System 1 endpoint, pool order, cost cap) that can be
  built in slices.
- G5. Crash safety for the decisions page as a named slice.

Non-goals:

- N1. Implementing any slice here. Each slice gets its own issue and queue item.
- N2. Redesigning the decisions page UI (P2.2 shipped; changes are follow-ups).
- N3. Rewriting historical decision files. They are linked, not moved (per `doc/decisions/README.md` §1).
- N4. Choosing a model vendor. Vendor comparisons belong to the decision-model research slice.
- N5. Changing who may rule. The owner remains the decider for product rulings.

## 3. Alternatives

A. **Do nothing.** Keep the register as the only record. Cost: rulings keep going missing and P2.3 grows
   stale. Rejected, because the register already shows the loss.

B. **One GitHub issue per open ruling.** Each P2.3 item becomes an issue. Strength: visible,
   assignable. Weakness: about 20 issues for short questions, and the queue would need claim rules for
   them. Chosen only for rulings that need implementation work.

C. **Single living ledger file plus FDR records (chosen).** The ledger lists open rulings with state
   and decider. Each answer produces an FDR, or an amendment to an FDR. Strength: keeps the answer
   trail and the open list apart. Weakness: two places to keep in sync; the design needs a rule for
   that (slice 2).

D. **Put everything in the decisions page.** The page becomes the ledger. Strength: one UI. Weakness:
   the page is for batched answers, and it is not a durable record; it would need to carry FDR
   semantics it was not built for.

E. **Decision models as a single fixed model.** One System 1 model, no pools. Simple, but it drops
   the cost and availability goals of the brief. Kept as the fallback if pools prove too complex.

## 4. Chosen approach

### 4.1 Structure

- This document is the family index. It owns the list of P2 children and their status.
- The open-rulings ledger is one file: `pm-stack/decisions-ledger.md` (new, owned by slice 2). Each row:
  ledger id, subject, decider, state (open | answered | dropped), FDR or issue link, last checked date.
- Answers are recorded as FDR records in `doc/decisions/`. A changed answer supersedes the old FDR.

### 4.2 Contracts

- Ledger row fields are fixed. A row is never deleted; it moves to `answered` or `dropped` with a
  reason.
- The hourly questions job reads only `open` rows. It does not write answers.
- The decisions page reads and writes drafts through the existing endpoint. The crash fix does not
  change that contract (slice 3).

### 4.3 Data model

- Ledger: one row per ruling, keyed by ledger id (existing ids such as `fleet-tiers-2230` are kept).
- Decision set: a group of rows answered in one push. Existing `decisions-2026-11-11` is the first set.

### 4.4 Security and capability

- Decisions are a control surface for fleet behaviour. Only the owner's identity may answer. Lanes may
  propose rows and read them, but may not mark a row answered.
- Drafts store user input in server-side and IndexedDB storage. Drafts are not secrets, but they
  must not carry tokens.
- Decision-model pools (P2 children, slice 6) must not send owner data to a provider outside the
  allowed list. The provider list is an open question (Q6).

### 4.5 Decision-model pools (from the 2026-11-13 supplement)

- Expose a System 1 endpoint. Chat-shaped traffic keeps its existing endpoints (opencode#1146).
- Pool order: free models first, then plan models, then pay-as-you-go. A cap on pay-as-you-go is set
  by the included Cloudflare usage. Dispatch disengages when the cap is hit.
- Cost comparison is per provider, with data, before a default is set.

## 5. Build slices (dependency order)

Each slice has its own acceptance criteria and should fit one lane session.

| # | Slice | Depends on | Acceptance |
|---|-------|------------|------------|
| S1 | This design, indexed from #12951 | none | This doc passes the 9-point check; #12951 links it |
| S2 | Create `pm-stack/decisions-ledger.md` from the P2.3 table; add the sync rule to the hourly job | S1 | Every P2.3 item has a row with decider and state; job reads only `open` rows |
| S3 | Decisions-page crash fix (#8777): self-heal on SIGTERM for aggregator and dispatch | S1 | Kill test: SIGTERM both units; both restart within 30 s; no draft lost |
| S4 | FDR for each answered ruling in S2 that governs fleet behaviour | S2 | Each such ruling has an FDR or an amendment; index updated in `doc/decisions/README.md` |
| S5 | Decision-model research: cost and capability per provider | S1 | Table with cost per decision, capability notes, source link per row |
| S6 | System 1 endpoint and pool order, behind a flag | S5 | Contract test passes for one provider; cap test: calls stop at the included amount |
| S7 | Retire the register's P2 table in favour of the ledger and this index | S2, S4 | Register links to ledger; no P2.3 item lacks a row |

## 6. Rollout and rollback

- Rollout: S1 and S2 are docs only and roll out by PR. S3 rolls out behind its existing units, with a
  restart policy change that can be reverted by one unit file change. S6 rolls out behind a flag,
  off by default.
- Rollback: docs revert by PR. S3 revert restores the previous unit file. S6 flag off returns to the
  current behaviour; no pool data is migrated.
- Data: the ledger is append-only in practice (rows change state, never vanish). Rollback keeps it.

## 7. Verification

- S1: read-back check of this doc against the nine standard sections, and a link from #12951.
- S2: a script or review pass that counts P2.3 items against ledger rows. Target: zero missing.
- S3: a kill test on both units, as above, run on the laptop with `fleet shell`.
- S4: each FDR lists its source ruling and date; the index test for FDR files already exists.
- S5: cost numbers are reproducible from the cited pricing source, with a date.
- S6: contract test for the System 1 response shape, and a cap test that stops calls at the included
  amount.
- Live proof for S3 and S6 is required before either is marked done. Documentation alone does not
  close them.

## 8. Open questions

Each question names a decider. Where the owner must rule, the question is also in the ledger (S2).

- Q1. Should each P2.3 ruling get its own issue, or stay as a ledger row? Decider: owner (product).
  Working default: ledger row; issue only when implementation work exists.
- Q2. Who may write `answered` on a ledger row? Decider: owner. Working default: owner identity only.
- Q3. Is the crash in #8777 a unit-policy bug or an external signal? Decider: owner for the signal;
  the lane owning S3 for the unit fix. Working default: fix the unit policy now; keep the witness armed.
- Q4. Is a System 1 endpoint worth building if DeepSeek Flash is cheaper per call? Decider: owner,
  after S5 data. Working default: S5 first; no endpoint until data exists.
- Q5. What is the pay-as-you-go cap for decision calls? Decider: owner. Working default: the included
  Cloudflare Workers amount, as the brief says.
- Q6. Which providers may receive decision traffic? Decider: owner. Working default: providers already
  allowed for chat traffic.
- Q7. Does the hourly job keep its current cadence once the ledger exists? Decider: owner. Working
  default: keep current cadence.

## 9. Recorded links

- Source issue: #12951 (link this doc in a comment once the PR merges).
- Queue item: 21540.
- Related: #8986, #13871, #13873, #8831, #8766, #8777, #9180, opencode#1146, opencode#1410.
