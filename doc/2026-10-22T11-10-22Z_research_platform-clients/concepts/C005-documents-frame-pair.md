# C005: Documents as a session frame pair

[Main reference](../main.md#c005)

## Relevant transcript excerpts

<!-- workflow:excerpt C005 203 206 -->

[Transcript lines 203-206](../transcript.txt#L203-L206)

```text
  records renderer with paging, and a **local read-only document viewer** over
  the server's blob store (`list_public_documents`, `read_public_document`:
  public names only, `.txt/.md/.markdown`, blob `Complete`, ≤64 KiB, valid
  UTF-8).
```

<!-- workflow:excerpt C005 234 234 -->

[Transcript lines 234-234](../transcript.txt#L234-L234)

```text
**Keep regardless of the decision:** the whole document stack, the records
```

<!-- workflow:excerpt C005 243 246 -->

[Transcript lines 243-246](../transcript.txt#L243-L246)

```text
Known gaps to fix while reconciling: `/records` auto-drains **all** pages with
only a per-page 384 KiB cap (no total cap); document handlers write to the
channel inside `recv` rather than through `send`; documents read the server's
local blob store, not a per-world store.
```

<!-- workflow:excerpt C005 388 388 -->

[Transcript lines 388-388](../transcript.txt#L388-L388)

```text
| Read the docs of a world | Line | `/documents`, `/document guide.md` — read-only, bounded |
```

<!-- workflow:excerpt C005 406 418 -->

[Transcript lines 406-418](../transcript.txt#L406-L418)

```text
- Move PR #211's document stack (`list_public_documents`,
  `read_public_document`, `DocumentError`, name filters) and
  `render_records_value` + the `"records"` reply arm into `world_ssh.rs` on
  top of `main`, keeping `main`'s `"view"` arm untouched
  (`WorldSurface::Screen.render_ansi`).
- Keep `WorldEscape` (escape swallowing) and the length/UTF-8 bounds for any
  text path; keep the injection tests.
- Fix the known gaps while moving: a **total** records render cap (not just
  per-page 384 KiB), and route document output through `send`.
- Decision to record: documents read the **server's** blob store today. If
  web and CLI should see them too, a `documents`/`document` **frame pair** (like
  `records`) belongs in `world_session.rs`; if SSH-only is fine, keep it local.
  Recommend the frame pair for parity, but it can wait.
```

<!-- workflow:excerpt C005 432 432 -->

[Transcript lines 432-432](../transcript.txt#L432-L432)

```text
  script); `/document …` → the local blob path (or the new frame);
```

<!-- workflow:excerpt C005 444 444 -->

[Transcript lines 444-444](../transcript.txt#L444-L444)

```text
  document 64 KiB / list 512 items / 32 KiB; records total cap new. Chat and
```

<!-- workflow:excerpt C005 658 658 -->

[Transcript lines 658-658](../transcript.txt#L658-L658)

```text
agents; documents as a protocol frame pair; the Roc partial-line surface; then
```

<!-- workflow:excerpt C005 786 787 -->

[Transcript lines 786-787](../transcript.txt#L786-L787)

```text
3. **Documents** — SSH-local blob read (fast) or a `documents` frame pair for
   web/CLI parity (cleaner)?
```

## Normalized purpose

Takes the world documents path: today an SSH-local read of the server's blob store, and the proposed documents/document ClientFrame/ServerFrame pair in world_session.rs (like records) for web and CLI parity, with its bounds and the per-world store question.

## Initial thoughts

- Documents are read from the server's local blob store, over SSH only (L203-L206, L245-L246; world_ssh.rs:455); world_session.rs has no documents frame.
- A documents/document frame pair like records gives web and CLI parity; recommended, but it can wait (L415-L418, L658).
- Q3 (L786-L787) is open: SSH-local read (fast) or frame pair (cleaner).
- #548 holds the prerequisite gaps: document output through send and a per-world store (L244-L246). #535 tracks D1-040 after #548.
- Bounds to keep: 64 KiB per document, 512 items and 32 KiB per list (L444; world_ssh.rs:596-598).

## Inferred suggestions

- Inference: Decide which scope gates document reads once they go through the session driver and the #256 projection.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
