# id-tui-shell

Terminal (TUI) shell for `id` — more-shells v2 slice 2 (issue
#12818; design `doc/2026-11-23T03-17-14Z_design_more_shells_v2/`,
§2, from the native TUI client idea, issue #1071). A ratatui
client speaking the F13.1 shell contract
(`pkgs/id/contract/shell-contract.v1.json`): it covers the full
text-native workflow of a world session — join, read, act,
reconnect under the v1 history-identity rule — at parity with
the browser shell's text surface. It adds no frame kinds and
needs no host change.

This crate is a workspace member but deliberately **not** a
default member, like the other shells, so the ordinary `id`
build never acquires this shell's dependencies:

```sh
cargo build -p id            # no TUI shell — default members only
cargo build -p id-tui-shell  # the TUI shell
cargo test -p id-tui-shell   # capability-statement tests
```

## Status

- **Capability statement — recorded** (this slice, per decision
  D3 of the v2 outline: the claimed subset is recorded before
  implementation starts). See the section below.
- **Frame builders, session fold and reconnect — landed**
  (`src/frames.rs`, `src/session.rs`), with the recorded
  sessions `contract/sessions/tui.jsonl` and
  `tui-reconnect.jsonl` checked against the shell suite.
- **Three-panel text projection — landed** (`src/present.rs`):
  room/object selector, room/document and chatroom (#1071),
  a pure function of the session fold with every host string
  made escape-free. `tests/present.rs` pins the panels for the
  recorded sessions.
- **Renderer — landed** (`src/render.rs`): ratatui draws the
  three panels and the input line, with the chat kept on its
  newest lines and the cursor after the typed text.
  `tests/render.rs` checks the recorded session on ratatui's
  test backend.
- **Key handling — landed** (`src/keys.rs`): Tab moves focus
  between the panels; chat focus edits a line and Enter sends
  `chat`, document focus sends keys as `input` bytes and pastes
  as `paste`, all through the frame builders.
- **Session loop and terminal — landed** (`src/app.rs`,
  `src/term.rs`, `src/main.rs`): the `id-tui-shell` binary dials
  the world host the way `id world join` does, joins with the
  capability, and folds host frames and keys through `App`. When
  the stream drops it redials and resumes from its last applied
  sequence; a host refusal ends the session. `tests/app.rs`
  drives `App` with the recorded sessions.
- **Form answers — landed** (`src/form.rs`): while a form is in
  force, a chat line that is `/cancel` or a JSON object
  `{"action": "<id>", "values": {...}}` answers it, as in the
  SSH terminal presenter. The answer is checked against the
  form's field model before it is sent, and a rejection with a
  fresh token re-presents the form.

Run it against a world host:

```sh
cargo run -p id-tui-shell -- --node <node-id> --capability <cap> [--world <name>]
```

## Capability statement

Per the v2 outline (§2 and decision D3), this shell's
capability statement is recorded in
[`capability-statement.json`](capability-statement.json) before
implementation starts, stated plainly in the hardware shell's
style: the client frames it sends (`join`, `input`, `paste`,
`resize`, `chat`, `form_result`, `records`), the server frames
it presents (`snapshot`, `event`, `view`, `form_result_status`,
`records`, and `error` surfaced as a session refusal), the
capability providers it declares (none — no F13.7 provider is
named for the TUI in #1071, the outline, or #12818), its
reconnect rule (history identity; divergence resolves by
re-snapshot, never by merge), and what it excludes by name
(rich rendering, media, provider offers, world hosting and
program authoring, offline reading, agent-profile surfaces).

The claimed subset is the browser shell's text surface
(`pkgs/id/web/src/world.ts`, `world-form.ts`) — the parity
anchor #12818 names — not the native GUI shell's wider one:
no screen-region subscription, no semantic diffs, no provider
frames. `tests/capability_statement.rs` pins the statement to
the contract file and to the workflow the issue names, so the
record cannot drift from the contract it is a statement
against; when the builders and the session fold land, the
check grows the same builder/fold oracles the native shell's
statement check carries (#10928). The conformance run is
wired into the check — the `test-tui-shell` flake check runs
this crate's test suite.
