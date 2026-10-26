# C001: SSH command (line) mode and the #211 gaps

[Main reference](../main.md#c001)

## Relevant transcript excerpts

<!-- workflow:excerpt C001 29 44 -->

[Transcript lines 29-44](../transcript.txt#L29-L44)

```text
2. **One world PR remains: #211** (SSH chat and documents) — `OPEN`,
   `CONFLICTING`. Research shows exactly what it adds (§7) and where it
   conflicts: four regions in `SshIo`, all reconcilable. It is line-buffered
   input vs `main`'s raw keystroke forwarding. This is a product decision,
   not a technical blocker.
3. **The input decision is made (§5–§7): raw keystrokes are the interactive
   default; a line mode becomes a second, explicit entry path** for automation,
   agents and quick checks — selected deterministically (an SSH username prefix
   like the existing `explore:`, or a CLI flag), sharing one command grammar
   with `id world`. No content auto-detection. Long-term, agreed: every
   variation — raw/buffered × primary/alternate, split UI, dual channel —
   lands eventually, each for a different context (§13–§15). §7 is a concrete
   implementation plan grounded in file/line.
4. **What the modes are for (§6):** play, quick checks, terminal chat, doc
   reading, scripting and piping, agent/CI transcripts, fixture recording.
   The web client already runs the "split UI" shape (screen + chat form).
```

<!-- workflow:excerpt C001 46 48 -->

[Transcript lines 46-48](../transcript.txt#L46-L48)

```text
   tasks with measurable exit criteria (§8). Input modes appear in *neither*
   plan; SSH lives in Phase 9. Adding the modes is a new work item worth
   recording in the Phase 11 plan.
```

<!-- workflow:excerpt C001 53 55 -->

[Transcript lines 53-55](../transcript.txt#L53-L55)

```text
7. **Where next (§12):** decide the mode shape → reconcile #211 in three
   steps (keep-stack, then line mode as a separate entry, then optional
   toggle) → Phase 11 order F11.17, F11.18, F11.14, F11.5, F11.10 → CI slice
```

<!-- workflow:excerpt C001 130 146 -->

[Transcript lines 130-146](../transcript.txt#L130-L146)

```text
## 3. How input works today (code level)

The full keystroke path, from terminal to guest:

1. **SSH channel bytes** arrive at `SshClient::data` (`world_ssh.rs:310-330`)
   → `SshInputFrame::Bytes` into a 256-slot `mpsc` queue via `try_send` —
   never waits; **input is dropped on a full queue** (`world_ssh.rs:323-324`).
2. **`SshIo::recv`** (`world_ssh.rs:371-389`) wraps each batch as
   `{"type":"input","data_hex":<hex>}` (or `{"type":"resize",…}`); the join
   frame is synthesized from the SSH login (`world_ssh.rs:351-357`) —
   **username = world name, password = join capability**; an `explore:` user
   prefix opens the directory explorer instead (`world_ssh.rs:143-146`).
3. **`ClientFrame::Input`** (`world_session.rs:233-235`) is hex-decoded and
   dispatched in `handle_client_frame` (`world_session.rs:2256-2468`) to
   `world.input(token, data)`; `paste` (≤4096 bytes, no control chars) and
   `chat` take sibling paths; `resize` (1..=500 × 1..=200) is **ephemeral,
   never journaled** and re-sends a fitted `view`.
```

<!-- workflow:excerpt C001 178 189 -->

[Transcript lines 178-189](../transcript.txt#L178-L189)

```text
Existing seams a line mode can use:

- **`SessionIo`/`Inbound`** (`world_session.rs:66-88`) — transport abstraction
  SSH already implements.
- **`WorldClient`** (`world_net.rs:157-964`) — wraps every request/response
  pair for out-of-band queries (`logs`, `stats`, `record`, `replay` use it).
- **The explorer's line editor** — `Typing` (`world_ssh.rs:615-677`) with
  echo, backspace, Enter, ESC-discard, 1024-byte cap, and the
  `directory_view::parse_line` grammar (`directory_view.rs:612-738`) is a
  working in-repo precedent for a line shell over the same channel.
- **`id world join`'s pump** (`commands/world.rs:781-798`) — a tiny client-side
  line grammar today: `/quit`, `/input <hex>`, else chat.
```

<!-- workflow:excerpt C001 194 253 -->

[Transcript lines 194-253](../transcript.txt#L194-L253)

```text
## 4. PR #211: exactly what it adds

Branch `origin/platform-ssh-world`, +761/−45 across 4 files (5 commits over
merge-base `292d3c7a`; PR claims "17 passed" in the focused `world_ssh` suite).

- **`world_ssh.rs` (+714/−16):** `WorldLineBuffer` (submit on CR/LF, CRLF once,
  backspace editing, 1024-byte cap with overlong-line discard, **no echo**),
  an escape-sequence swallowing state machine (`WorldEscape`: CSI/SS3/OSC/DCS/
  APC/PM/SOS — kills OSC 52 clipboard injection), `world_command` parser, a
  records renderer with paging, and a **local read-only document viewer** over
  the server's blob store (`list_public_documents`, `read_public_document`:
  public names only, `.txt/.md/.markdown`, blob `Complete`, ≤64 KiB, valid
  UTF-8).
- **Commands — correction:** there is **no `/chat`**. Any plain line becomes a
  `chat` frame (implicit chat). The set is: `/input <hex>`, `/records
  [prefix]`, `/documents [prefix]`, `/document <name>`, `/help`, `/quit`.
- **`commands/serve.rs` (+9/−3):** passes the blobs `store_handle` into
  `world_ssh::serve`. `blob_gate.rs` is rustfmt-only. One doc record gets a
  rewritten SSH section.
- **Tests (8):** e2e chat + records on a vt100 terminal; fragmented-packet and
  CRLF handling; command mapping and `/input` hex rules; overlong discard;
  OSC 52 drop; JSON-escaped bounded records rendering; document name/size/
  binary limits (incl. `../`, newline-in-name); e2e document listing/viewing
  with escape neutralization. The sandbox shell test was changed to send
  `/input 696e63\r` — proof the PR changes the input contract.

**Conflicts with `main` (merge-tree dry run): four regions, all in `SshIo`:**

1. Construction in `shell_request`: `main`'s `first_resize` vs the PR's
   `capability/lines/commands/records_prefix`.
2. `struct SshIo`: `main`'s `mpsc::Receiver<SshInputFrame>` (Bytes/Resize) vs
   the PR's `Receiver<Vec<u8>>` + command fields.
3. `SessionIo::recv`: `main` forwards resize frames (per-session viewport);
   the PR's command loop would **drop them** — semantic conflict, not just
   textual.
4. `SessionIo::send`'s `"view"` arm: `main` renders `value["surface"]` via
   `WorldSurface::Screen(...).render_ansi()`; the PR replaces it with a
   clear-repaint + prompt banner and adds `"snapshot"`/`"records"` arms. A
   naive merge loses surface rendering.

**Keep regardless of the decision:** the whole document stack, the records
renderer and paging arm, the blobs plumbing, `WorldEscape` + length/UTF-8
safety + the injection test, `world_command`'s validation rules.

**Inherently line-mode:** `WorldLineBuffer`, implicit chat (needs an explicit
`/chat` in raw mode), `/input <hex>` (a workaround for line mode — in raw mode
keystrokes go direct), the `world> ` prompt banners appended to view repaints
(collide with full-screen surface repaints), Ctrl-C/Ctrl-D quit semantics.

Known gaps to fix while reconciling: `/records` auto-drains **all** pages with
only a per-page 384 KiB cap (no total cap); document handlers write to the
channel inside `recv` rather than through `send`; documents read the server's
local blob store, not a per-world store.

## 5. SSH input: the design decision

### Three things that get conflated

- **Input routing** — raw keystrokes forwarded one by one, versus line-buffered
  text collected until Enter.
```

<!-- workflow:excerpt C001 262 277 -->

[Transcript lines 262-277](../transcript.txt#L262-L277)

```text
### Does the local client update the screen while you type?

| Mode | Who draws while you type | What the server receives |
|---|---|---|
| Raw (world default) | Nobody; the world redraws on each input | every keystroke |
| Buffered line, local echo | the local client, like a shell prompt | the whole line on Enter |
| Buffered line, server echo | the server draws the prompt and echoes | keystrokes, echoed and drawn by the server |
| PTY canonical / cooked | the kernel line discipline buffers and echoes | the line on Enter |

Local echo is snappier but the server does not know what is typed, so
autocomplete and validation need streaming anyway. Server echo gives the
server everything as typed, at one round trip per keystroke unless batched.
The hybrid modern TUI line editors use over SSH: stream keystrokes for
intelligence, let the client draw locally. (PR #211 uses **no echo at all** —
it relies on the remote terminal's own echo, which is why it needs no
round-trips and why overlong/edited lines can desync the display.)
```

<!-- workflow:excerpt C001 317 337 -->

[Transcript lines 317-337](../transcript.txt#L317-L337)

```text
4. **Primary mode, line oriented.** Scrollback, buffered commands,
   line-oriented output; the world prints text lines, you type at a prompt.
   **Gains:** trivially scriptable — pipe in commands, parse output, tab
   complete, autocomplete, error hints, retries. What an agent or CI job
   wants. **Losses:** no real-time grid; output is a transcript.

### Mode-switch triggers

| Trigger | Deterministic | Needs client support | Notes |
|---|---|---|---|
| SSH username prefix (`cmd:` like `explore:`) | yes | no | mirrors the existing explorer pattern; picks the mode at login |
| Startup flag or subcommand | yes | no | `id world join --command-mode` for the CLI client |
| Prefix key (`Ctrl-P`, `~ Enter`) | yes | no | the in-session toggle; needs visible mode indicator |
| Dedicated key (`Ctrl-]`, `Ctrl-\`) | yes | no | no collision with editing keys, undiscoverable |
| Type `/` in raw mode | no | no | a game that wants to type `/` breaks — do not |
| Ctrl+C / interrupt | ambiguous | no | already used for cancel; do not overload |
| Client capability negotiation | yes | yes | client tells the server what it supports |

Two distinct switches: **session start** (line mode for automation/agents vs
raw interactive for humans) and **in-session** (entering/leaving command mode
while play stays up — the prefix-key and indicator work).
```

<!-- workflow:excerpt C001 353 379 -->

[Transcript lines 353-379](../transcript.txt#L353-L379)

```text
### Why allow both — and why not

Games need raw; operators and agents need commands. The risk is only in
coexistence:

- **Bad:** auto-detect ("if it looks like text, buffer it") — non-deterministic
  and untestable.
- **Good:** explicit, one-way routing. Mode chosen at login (or by a visible
  prefix-key toggle later); mode is visible; exit returns to raw.

Costs of both (not blockers): more protocol surface and tests; mode state can
mis-route input under load; two ways to send the same message complicates
auditing.

### Recommendation

1. **Raw keystrokes stay the interactive default.** Consider moving the
   renderer to the alternate screen as a small, separate improvement.
2. **Line mode as a separate entry path**, same command grammar as the CLI:
   pipe in commands, one-offs, tab completion. No world screen required.
3. **One trigger at first:** the login-time choice (SSH username prefix or CLI
   flag). Add the prefix-key toggle only if people want to switch mid-session.
4. **Defer in-session buffered typing (the Roc partial-line surface)** until
   the line mode exists and the command grammar is known.

Do not auto-detect from content. Do not build two SSH channels until someone
has a real client for it.
```

<!-- workflow:excerpt C001 381 393 -->

[Transcript lines 381-393](../transcript.txt#L381-L393)

```text
## 6. What the modes are for

| Use case | Mode | What it looks like |
|---|---|---|
| Play a game (`territory`, tic-tac-toe) | Raw | Keystrokes go straight to the program; full-screen grid redraws |
| Quick check ("what's the state?") | Line | `stats`, `records`, `logs` at a prompt — or a CLI one-off `id world stats` |
| Chat from a terminal without playing | Line | Plain line = chat (PR #211's implicit chat is exactly right here) |
| Read the docs of a world | Line | `/documents`, `/document guide.md` — read-only, bounded |
| Power user mid-game | Raw + later toggle (shape 3) or split UI (shape 1) | Prefix key for a command, back to play |
| Scripting / piping | Line | `printf 'records\nquit\n' \| id world join …`, or CLI verbs run once and exit |
| Agents and CI | Line | Deterministic transcript output; same grammar as the CLI; later `--json` output; perfect for smoke tests against a running world |
| Fixture recording / replay | CLI | `id world record` / `replay` (on `origin/main`) driven by the same grammar |
| Accessibility / slow links | Line | Transcript output works where a 60 fps grid does not |
```

<!-- workflow:excerpt C001 399 454 -->

[Transcript lines 399-454](../transcript.txt#L399-L454)

```text
## 7. How to add the modes (implementation plan)

Grounded in the code surveyed in §3–§4. Three steps land #211's value without
pre-committing to dual channel.

### Step 1 — Land the mode-neutral keep-stack (small PR)

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

### Step 2 — Line mode as an explicit second entry (the core PR)

- **Mode selection at login.** Mirror `explore:` (`world_ssh.rs:143-146`):
  a `cmd:` username prefix (or bare-name default staying raw) selects
  `ShellMode::Command`; `explore:` keeps the directory. For the CLI client,
  `id world join --command-mode` (or plain stdin detection by **explicit
  flag only**).
- **Where the parser sits.** Before `SshIo::recv`'s JSON wrapping
  (`world_ssh.rs:380-383`): a `WorldLineBuffer`-style accumulator converts
  lines into **real `ClientFrame`s** on the existing session — plain line →
  `chat{text}`; `/records [prefix]` → `records{prefix}`; `/input <hex>` →
  `input{data_hex}` (kept as an escape hatch for sending game input from a
  script); `/document …` → the local blob path (or the new frame);
  `/stats`, `/logs` → the same frames `WorldClient` uses. **Dispatch through
  the session driver, not around it** — that keeps the #256 projection
  (chat hidden without `history`, seed redaction) authoritative for every
  path.
- **Keep `resize` frames flowing in command mode** (PR #211's semantic bug);
  the transcript can still fit line widths to the viewport.
- **Output is transcript mode:** no `\x1b[H\x1b[2J`; `view` frames print
  appended lines (or are suppressed in favor of query replies); prompt `id> `
  or `world> ` after each reply; `error` frames keep the control-stripping
  sanitizer (`world_ssh.rs:681-686`).
- **Bounds and journaling:** line buffer ≤1024 bytes (PR #211's discard rule);
  document 64 KiB / list 512 items / 32 KiB; records total cap new. Chat and
  input lines become durable events (existing paths); **partial lines and
  mode state are never journaled** (same rule as `resize`).
- **Tests:** port PR #211's 8 tests onto the reconciled code; add one that
  asserts resize works in command mode, one that the `history` scope still
  gates chat in line-mode `records`, and the existing vt100 e2e. Keep
  `a_shell_plays_the_world_and_repaints_each_view` sending raw bytes (the
  contract difference between the modes *is* the feature).
- **Docs:** `world_ssh.rs` module doc (username grammar: world name /
  `explore:` / `cmd:`), the arch doc record, and the Phase 11 plan gains a
  task (see §8).
```

<!-- workflow:excerpt C001 492 492 -->

[Transcript lines 492-492](../transcript.txt#L492-L492)

```text
- **#211** — SSH worlds: awaiting the mode decision; reconcilable per §7.
```

<!-- workflow:excerpt C001 505 505 -->

[Transcript lines 505-505](../transcript.txt#L505-L505)

```text
- **Input modes** — not in either plan; add as a Phase 11 task when landed.
```

<!-- workflow:excerpt C001 571 571 -->

[Transcript lines 571-571](../transcript.txt#L571-L571)

```text
| PR #211 | feat(ssh): add chat and document viewing to worlds | OPEN, CONFLICTING; four `SshIo` regions; reconcilable per §7 |
```

<!-- workflow:excerpt C001 629 631 -->

[Transcript lines 629-631](../transcript.txt#L629-L631)

```text
5. Said PR #211 has a `/chat` command. It does not: plain lines chat
   implicitly; commands are `/input`, `/records`, `/documents`, `/document`,
   `/help`, `/quit`.
```

<!-- workflow:excerpt C001 641 648 -->

[Transcript lines 641-648](../transcript.txt#L641-L648)

```text
**Immediate (decisions + #211):**

1. Decide the mode shape — recommended: raw default + line mode as a separate
   entry (login-time choice), shared grammar (§5–§7).
2. Reconcile #211 in three steps: keep-stack PR → line-mode PR → optional
   toggle. Fix the resize-drop and total-records-cap gaps while moving.
3. Housekeeping: fast-forward worktrees to `origin/main`; close or re-scope
   #230; add an input-mode task to the Phase 11 plan (it is in neither plan).
```

<!-- workflow:excerpt C001 748 749 -->

[Transcript lines 748-749](../transcript.txt#L748-L749)

```text
- **Tier 3 — dumb or slow:** the line mode (§5-§7) *is* the degraded
  presentation — transcript output, no grid.
```

<!-- workflow:excerpt C001 779 785 -->

[Transcript lines 779-785](../transcript.txt#L779-L785)

```text
## 16. Open questions for you

1. ~~Mode shape~~ **Decided:** raw default + separate line-mode entry, shared
   grammar. All variations (raw/buffered × primary/alternate, split UI, dual
   channel) land eventually, each for a different context (§13–§15).
2. **Line-mode chat UX** — keep PR #211's implicit plain-line-chat in line
   mode (recommended), or require explicit `/chat` everywhere for symmetry?
```

## Normalized purpose

Takes the input decision (raw default, line mode as a second explicit entry chosen at login), how input flows today, what PR #211 added, its conflicts and known gaps, the mode use cases and §7 steps 1 and 2. #211 merged on 2026-10-20 after the report, so this concept now covers checking what shipped and closing the remaining gaps.

## Initial thoughts

- #211 merged on 2026-10-20 (be748a3b) with the `cmd:` prefix and ShellMode::Command (pkgs/id/src/world_ssh.rs:37, :122); the report still calls it open and conflicting (L29-L31, L492, L571).
- Resize now flows in command mode (world_ssh.rs:576-582), which closes the semantic gap L226-L228 and L437 describe.
- The records and document gaps of L243-L246 are open in #548: the only cap is the 384 KiB per-render constant (world_ssh.rs:595), and document output is written inside recv (world_ssh.rs:453-482).
- Input is still dropped when the 256-slot queue is full (L134-L136; world_ssh.rs:354), tracked in #549.
- Command-mode views still clear and repaint with a `world> ` banner (world_ssh.rs:560-566), not the appended transcript L439-L442 planned; the collision L240-L241 named remains.
- Input modes appear in neither plan (L46-L48, L505, L648); #550 tracks recording the decision and the username grammar (world name, `explore:`, `cmd:`).

## Inferred suggestions

- Inference: S1 should start by diffing the report's §7 plan (L399-L454) against what be748a3b shipped, because the report predates the merge.
- Inference: The drop policy for a full input queue (#549) applies to raw mode as well as command mode.
- Inference: #550 is the natural first deliverable: it edits the SSH section of the roc worlds architecture record and the Phase 11 plan.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
