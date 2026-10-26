# C006: One command grammar and --json output

[Main reference](../main.md#c006)

## Relevant transcript excerpts

<!-- workflow:excerpt C006 36 38 -->

[Transcript lines 36-38](../transcript.txt#L36-L38)

```text
   agents and quick checks — selected deterministically (an SSH username prefix
   like the existing `explore:`, or a CLI flag), sharing one command grammar
   with `id world`. No content auto-detection. Long-term, agreed: every
```

<!-- workflow:excerpt C006 182 183 -->

[Transcript lines 182-183](../transcript.txt#L182-L183)

```text
- **`WorldClient`** (`world_net.rs:157-964`) — wraps every request/response
  pair for out-of-band queries (`logs`, `stats`, `record`, `replay` use it).
```

<!-- workflow:excerpt C006 186 189 -->

[Transcript lines 186-189](../transcript.txt#L186-L189)

```text
  `directory_view::parse_line` grammar (`directory_view.rs:612-738`) is a
  working in-repo precedent for a line shell over the same channel.
- **`id world join`'s pump** (`commands/world.rs:781-798`) — a tiny client-side
  line grammar today: `/quit`, `/input <hex>`, else chat.
```

<!-- workflow:excerpt C006 207 209 -->

[Transcript lines 207-209](../transcript.txt#L207-L209)

```text
- **Commands — correction:** there is **no `/chat`**. Any plain line becomes a
  `chat` frame (implicit chat). The set is: `/input <hex>`, `/records
  [prefix]`, `/documents [prefix]`, `/document <name>`, `/help`, `/quit`.
```

<!-- workflow:excerpt C006 238 240 -->

[Transcript lines 238-240](../transcript.txt#L238-L240)

```text
**Inherently line-mode:** `WorldLineBuffer`, implicit chat (needs an explicit
`/chat` in raw mode), `/input <hex>` (a workaround for line mode — in raw mode
keystrokes go direct), the `world> ` prompt banners appended to view repaints
```

<!-- workflow:excerpt C006 319 321 -->

[Transcript lines 319-321](../transcript.txt#L319-L321)

```text
   **Gains:** trivially scriptable — pipe in commands, parse output, tab
   complete, autocomplete, error hints, retries. What an agent or CI job
   wants. **Losses:** no real-time grid; output is a transcript.
```

<!-- workflow:excerpt C006 371 372 -->

[Transcript lines 371-372](../transcript.txt#L371-L372)

```text
2. **Line mode as a separate entry path**, same command grammar as the CLI:
   pipe in commands, one-offs, tab completion. No world screen required.
```

<!-- workflow:excerpt C006 375 376 -->

[Transcript lines 375-376](../transcript.txt#L375-L376)

```text
4. **Defer in-session buffered typing (the Roc partial-line surface)** until
   the line mode exists and the command grammar is known.
```

<!-- workflow:excerpt C006 386 386 -->

[Transcript lines 386-386](../transcript.txt#L386-L386)

```text
| Quick check ("what's the state?") | Line | `stats`, `records`, `logs` at a prompt — or a CLI one-off `id world stats` |
```

<!-- workflow:excerpt C006 390 392 -->

[Transcript lines 390-392](../transcript.txt#L390-L392)

```text
| Scripting / piping | Line | `printf 'records\nquit\n' \| id world join …`, or CLI verbs run once and exit |
| Agents and CI | Line | Deterministic transcript output; same grammar as the CLI; later `--json` output; perfect for smoke tests against a running world |
| Fixture recording / replay | CLI | `id world record` / `replay` (on `origin/main`) driven by the same grammar |
```

<!-- workflow:excerpt C006 395 397 -->

[Transcript lines 395-397](../transcript.txt#L395-L397)

```text
Design rule that follows: **the line grammar should mirror the `id world`
subcommands** (`chat`, `records`, `stats`, `logs`, `caps`, `quit`) so the same
sentence works as a CLI one-off, a piped script line, and an SSH command.
```

<!-- workflow:excerpt C006 422 426 -->

[Transcript lines 422-426](../transcript.txt#L422-L426)

```text
- **Mode selection at login.** Mirror `explore:` (`world_ssh.rs:143-146`):
  a `cmd:` username prefix (or bare-name default staying raw) selects
  `ShellMode::Command`; `explore:` keeps the directory. For the CLI client,
  `id world join --command-mode` (or plain stdin detection by **explicit
  flag only**).
```

<!-- workflow:excerpt C006 433 433 -->

[Transcript lines 433-433](../transcript.txt#L433-L433)

```text
  `/stats`, `/logs` → the same frames `WorldClient` uses. **Dispatch through
```

<!-- workflow:excerpt C006 463 464 -->

[Transcript lines 463-464](../transcript.txt#L463-L464)

```text
3. **`--json` transcript output** for agents/CI; then scripted round-trips
   need no terminal at all.
```

<!-- workflow:excerpt C006 470 475 -->

[Transcript lines 470-475](../transcript.txt#L470-L475)

```text
### What the CLI gains along the way

The `id world join` pump (`commands/world.rs:781-798`) grows the **same
grammar** (`/records`, `/stats`, `/chat …`, `/quit`) so piped scripts and the
SSH line mode are interchangeable; one-off verbs stay as subcommands
(`id world stats NODE` already exists).
```

<!-- workflow:excerpt C006 629 631 -->

[Transcript lines 629-631](../transcript.txt#L629-L631)

```text
5. Said PR #211 has a `/chat` command. It does not: plain lines chat
   implicitly; commands are `/input`, `/records`, `/documents`, `/document`,
   `/help`, `/quit`.
```

<!-- workflow:excerpt C006 657 658 -->

[Transcript lines 657-658](../transcript.txt#L657-L658)

```text
**Later:** the optional in-session toggle; `--json` transcript output for
agents; documents as a protocol frame pair; the Roc partial-line surface; then
```

<!-- workflow:excerpt C006 784 785 -->

[Transcript lines 784-785](../transcript.txt#L784-L785)

```text
2. **Line-mode chat UX** — keep PR #211's implicit plain-line-chat in line
   mode (recommended), or require explicit `/chat` everywhere for symmetry?
```

## Normalized purpose

Takes the design rule that one line grammar mirrors the id world subcommands across SSH command mode, the id world join stdin pump and CLI one-offs, the explicit --command-mode flag, the chat syntax question, and later --json transcript output for agents and CI.

## Initial thoughts

- Design rule: the line grammar mirrors the id world subcommands (chat, records, stats, logs, caps, quit), so one sentence works over SSH, in a pipe and as a CLI one-off (L395-L397).
- Today the surfaces differ: SSH world_command has /help, /quit, /records, /documents, /document and /input (world_ssh.rs:620-668); the join pump knows /quit and /input (pkgs/id/src/commands/world.rs:1005-1006); #546.
- Contradiction kept: L207-L209 and L629-L631 say plain lines chat and there is no /chat; L473 lists `/chat …` in the CLI grammar; Q2 (L784-L785) leaves implicit or explicit chat open and recommends implicit.
- `--command-mode` for `id world join` is explicit-flag only (L422-L426); pkgs/id/src/cli.rs has no such flag.
- `--json` output for agents and CI comes later (L463-L464, L657; #563, after #546).

## Inferred suggestions

- Inference: `id world join` already prints server frames as JSON lines (commands/world.rs:992-994), so the CLI may already have part of --json.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
