# D1-040 — Documents as a session frame pair (web/CLI parity)

Design for tracker item D1-040 in
[#535](https://github.com/developing-today/code/issues/535). Source: RPT §7
Step 1 'Decision to record'; RPT §16 Q3; RPT §12 Later. Prerequisite
[#548](https://github.com/developing-today/code/issues/548) (D1-039
reconciliation gaps) is closed. One scope correction to the tracker's
framing, recorded in Decision 1 below: #548 did **not** deliver a per-world
document store — the 2026-10-24 decision record
(`doc/2026-10-24T20-18-37Z_decision_ssh_input_modes`) explicitly defers it,
and documents today are read from the **shared server blob store**. This
design is written against that real baseline.

## Intent

The public documents (the named read-only `.txt`/`.md`/`.markdown` files
in the server's blob store) can be read today only from the SSH shell. The web
and CLI clients speak the world-session frame protocol and have no frame for
documents, so the same content is unreachable there. Records — the world's
structured key/value data — already crossed this exact gap with a
`records`/`records_ticket` frame pair. This design gives documents the same
treatment: a `documents`/`document` ClientFrame/ServerFrame pair in the
world-session protocol, modelled on `records`, with the SSH shell becoming
one rendering client of the frames instead of the only reader.

## Current state (verified on main, 2026-11-01)

- `/documents [prefix]` and `/document <name>` are answered by the SSH shell
  itself. In `pkgs/id/src/world_ssh.rs`, `command_frame` returns `None` for
  `WorldCommand::Documents` / `WorldCommand::Document` — they are in the
  "commands the shell answers itself" set — and `SshIo::recv` (Command arm)
  reads the server's iroh-blobs store directly via `list_public_documents`
  and `read_public_document`. No session frame is involved, so no other
  client can ask.
- The serving rules live in those two functions and are the contract any
  frame version must preserve:
  - names: `is_public_document_name` (non-empty, not an internal tag, no
    `/` or `\`); reads additionally require `is_viewable_document_name`
    (UTF-8 text extensions `.txt`/`.md`/`.markdown`);
  - bounds: `MAX_DOCUMENT_BYTES` = 64 KiB per document,
    `MAX_DOCUMENT_LIST_BYTES` = 32 KiB and `MAX_DOCUMENT_LIST_ITEMS` = 512
    per listing, with a `[listing truncated]` marker;
  - read errors are typed (`DocumentError`: InvalidName, Missing, TooLarge,
    Binary, Unavailable) and the rendered read is headed
    `--- {name} ({size} bytes; read-only) ---`.
- The `records` model this design copies, in
  `pkgs/id/src/world_session/types.rs` and `mod.rs`:
  - `ClientFrame::Records { capability, prefix, after }` and
    `ServerFrame::Records { sequence, records, next_after }`, serde tag
    `type`, snake_case (`records`);
  - `records_session` authorizes with the join capability
    (`world.participant_id(token)`, else `join denied`), pages the world's
    records at `MAX_RECORDS_PAGE_BYTES` (768 KiB), and returns `next_after`
    as the exclusive continuation key;
  - the SSH shell renders pages with a drain pattern: `command_frame` emits
    `records` and `records_continue` frames and `SshIo` accumulates up to
    `MAX_RECORDS_TOTAL_BYTES` before marking truncation.
- Frame names are a contract: `pkgs/id/src/shell_contract/tests.rs` names
  every `ServerFrame` variant exhaustively ("a new variant does not compile
  until it is named here"), and the web client's contract tests
  (`pkgs/id/web/src/world-contract.test.ts`) exercise the same frame shapes.

## Decision

1. **Scope: server-wide parity, stated plainly.** The frame pair exposes
   exactly the document set the SSH shell exposes today — the public tags
   of the shared server blob store — no more, no less. Per-world document
   scoping is **not** part of this design: it waits on the per-world
   document store, which the 2026-10-24 decision record defers as future
   work. The alternative — describing the frames as world-scoped while
   serving server-wide content — would document a semantics the code does
   not have and invite an implementation to expose content under a false
   locality claim. When per-world storage lands, it narrows what the
   store returns; the frame shapes in this design do not change.
2. **Two client frames, mirroring the two commands.**
   - `ClientFrame::Documents { capability, prefix, after }` — page the
     server store's public document names, filtered by `prefix`, continuing after
     the name `after` (exclusive). Wire type `documents`.
   - `ClientFrame::Document { capability, name }` — read one document.
     Wire type `document`.
3. **Two server frames.**
   - `ServerFrame::Documents { names, next_after }` — one page of names in
     sorted order, `next_after` present when more names follow. Unlike
     `records` there is deliberately **no `sequence`**: records are world
     state with a journal sequence; the document listing is a point-in-time
     snapshot of the blob store's tag set, which has no sequence to report.
     Claiming one would be a false promise.
   - `ServerFrame::Document { name, size, text }` — the document's UTF-8
     text, its byte size, and its name echoed for correlation. The
     `--- {name} ({size} bytes; read-only) ---` header is a *rendering*:
     each client formats its own (SSH reproduces today's header byte-for-
     byte); it is not part of the payload.
4. **Authorization is the records rule, unchanged.** Both frames carry the
   join capability; the server checks `participant_id` and answers
   `join denied` otherwise. Documents are participant-public read-only content (the server-wide public set) —
   exactly the audience that can already read them over SSH — so no admin
   token and no new scope. (Rejected alternative: gating on a new
   `documents` capability scope. Nothing today distinguishes records
   readers from documents readers, and inventing the distinction here
   would break SSH parity on day one.)
5. **Bounds carry over, enforced server-side.** A single read is capped by
   the existing `MAX_DOCUMENT_BYTES` (64 KiB), which fits comfortably in
   one frame — no chunked transfer (contrast the module frames, which
   exist because modules are megabytes). List paging uses the records
   byte-page rule; the existing list item/byte caps remain the SSH
   renderer's drain limits, as `MAX_RECORDS_TOTAL_BYTES` already is for
   records. Name validation (`is_public_document_name`,
   `is_viewable_document_name`) runs in the session layer, never trusted
   from the client.
6. **Errors ride the existing error path.** Failures are `send_error`
   strings, one per `DocumentError` case, so SSH users see the same words
   they see today; the implementation issue pins the exact strings as
   contract tests.
7. **The serving logic moves; the SSH output does not.** The
   `list_public_documents` / `read_public_document` logic moves from `world_ssh.rs`
   into the session layer next to `records_session` (a `documents_session`
   and a document read handler in `pkgs/id/src/world_session/mod.rs`),
   reading the same blob store. `world_ssh.rs` keeps only rendering:
   `command_frame` gains `documents`, `documents_continue`, and `document`
   arms mirroring `records`/`records_continue`, `Documents`/`Document`
   leave the shell-answered set, and `SshIo` drains list pages with the
   same truncation accounting it uses for records. Rendered SSH output —
   the `public named files:` listing, the `[listing truncated]` marker,
   the read header, the `/help` text — is byte-identical before and after.
   One structural unknown the implementation must resolve first: the SSH
   shell currently holds the blobs store directly (`self.blobs`), while
   the session layer works through `WorldService`/`WorldHandle`; the
   implementation either exposes the store through the service or passes
   it to the session constructor the way the SSH layer receives it today.
8. **The contract grows in both places at once.** `documents` and
   `document` are named in `shell_contract` (`server_frame_type` +
   `SERVER_FRAME_TYPES` and the client-frame equivalents), the shared
   frame contract and the web contract tests are extended, and the web
   client gains a minimal documents panel (list + read) so parity is
   demonstrated, not just possible.

## Non-goals

- No writes: publishing, renaming, or deleting documents stays exactly
  where it is today. This pair is read-only, like the commands it lifts.
- No `documents_ticket`: records has a replication ticket because records
  replicate peer-to-peer; documents already live in the blob store, which
  has its own replication story. A ticket can be proposed separately if a
  client ever needs bulk sync.
- No per-world scoping (see Decision 1): the deferred per-world document
  store owns that question when it is built.
- No change to which documents exist or are public — #548's reconciliation
  (documents reading the server blob store, output through `send`) is an
  input to this design, not something it revisits.

## Test expectations for the implementation

- A parity test: the frame-served listing equals the SSH shell's listing
  for the same store (the server-wide set), so the two paths can never
  drift apart silently.
- Session tests mirroring the `records_session` tests: paging order,
  `prefix` filtering, `after` exclusivity, oversized-document refusal,
  non-viewable and missing names, `join denied` for a bad capability.
- Contract tests: both new frames named and shaped in `shell_contract`
  and in the web contract tests (accept and refuse cases).
- SSH shell tests: the existing document shell tests (world_ssh.rs, the
  `a_shell_lists_and_views_read_only_documents_from_its_server_store`
  family) pass against the frame-backed path with unchanged expected
  output; the direct-store tests for the listing/reading helpers move
  with the helpers into the session layer.

## Links

- Tracker: #535 item D1-040. Prerequisite: #548 (closed).
- Scope baseline: `doc/2026-10-24T20-18-37Z_decision_ssh_input_modes`
  ("Documents read the server blob store for now").
- Model: `records` frames, `pkgs/id/src/world_session/types.rs` and
  `records_session` in `pkgs/id/src/world_session/mod.rs`.
- Today's SSH-local path: `command_frame`, `list_public_documents`,
  `read_public_document` in `pkgs/id/src/world_ssh.rs`.
