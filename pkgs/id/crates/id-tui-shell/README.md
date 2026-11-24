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
- **Feature completion — open** on issue #12818: the frame
  builders, session fold, and three-panel presentation (room /
  object selector, room/document, chatroom — #1071) land in
  this crate on top of the statement, followed by conformance
  on the recorded session plus behavioral completion checks
  for the workflow, per the v1 gate.

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
