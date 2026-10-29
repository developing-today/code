# Future Plans

---

## **Warning For Agents & Onlookers**

> This is for future implementation.
>
> It's fine to read this file but don't make any significant decisions based on anything here.
>
> Anything here is just idle planning and is subject to change.

---

- Mail outbox removal decision: see #1795.

- Cursor presence UX remainder: see #1790.

---

- Web shell header/footer doubling: dropped — superseded by the Datastar/SPA rewrite (#1073); see the disposition on #1294. File fresh with a repro if it recurs.

- Web settings: see #1792.

---

- Structured logging: see #1791.

---

- Build hygiene: see #1797.

---

- ProseMirror wire format: see #1796.

---

- ensure website can collaboratively edit files with multiple clients connected to the same server, and that changes are reflected in real time across all clients. each document opened in the website should have it's own collaboration session. handle conflicts gracefully if multiple clients edit the same file at the same time. figure out how best to sync the state of the file back to the store, we don't want a new file for every character typed. maybe to startt we can just have a "save" button that syncs the current state of the file to the store, and then we can look into more real-time syncing options later, as well as some method of garbage collecting old file versions.
- ensure website looks nice and has whatever appropriate testing is needed for the features and for a webserver/website.

---

- ensure website has 1:1 capability with cli (can run equivalent of each command/flag except from a native browser ui)
- implement access to the repl from the website
- implement a method to switch the server one is connected to from the website

---

- Send files server-to-server: see #1794.
- Petnames: dropped — already implemented in `src/petname.rs`; the local-vs-shared scope remainder is in #1793. See the disposition on #1294.

---

- Tags provenance and uploaded-file origin: see #1793.

---

- make a tui using ratatui or whatever the highest performance tui rust crate is. aim for high performance, it should work locally without running other servers, or connect to the local server if it's running or connect to a remote server. use iroh for network communication not ssh, consider the most efficient way to handle this so that it is very performant. we want high fps, ability to make complex graphs, possibly transmit kitty image protocol images, coloring blocks, all the tui things you might want from something like ratatui, but ideally you wouldn't be sending all the terminal inputs from the server to the client. there should be a way to send a custom protocol in an iroh postcard or whatever, where you say what you want to do provide the new bytes, and then the client handles. like 'heres the bytes for the image, put the image in the screen at 64x 16y on the screen and let the image be 256\*256' and then the client can display the picture without the server needing to actually move the cursor, delete/redraw in the terminal, etc. server could say 'draw the level map at <blob> across the entire screen' and then the client would handle getting the blob itself. you wouldn't want a second round trip if they don't have it cached locally, so some thought would need to be put there. the tui should cover things like the website, except native access without upload boxes and with full control of the box. it should be a tui program, doesn't need to be ssh--- someone can ssh into the server and run the tui. maybe 3 panels, room/object selector/search/find/pin-favorites || the interactive room/document or other meta configuration or search pages || a chatroom for a given room or topic

---

- make a native gui using something like tauri, egui, tauri-egui, leptos, dioxus, iced, bevy, etc. try to remember we may later embed bevy project into the native app or embed the native app into bevy. it should do what the tui does, but faster and better.

---

- rust/js linters, clippy with all runs, run rustfmt, etc. (youtube video that mentioned what to run? there was another in addition to clippy..)

---

---

---

---

- if reconnect and new server has new css then reload new css/html
- header is too large, make it smaller and hide it except on hover when on an editor page. foot
- theme doesn't automatically update which is ok but when you hit alt+t and the local theme is same color as the next alt+t it doesn't do anything.
- when you make debug/testing logs dont delete them when you are done, just ensure they aren't still triggerred if info.
- victoriametrics
- footer for raw/media should match markdown. download should be in the bottom right of all per-page toolbars
- allow save back update hash, allow download saved which downloads last saved copy and download which downloads current file as-is. maybe allow download json for active documents too which is same as download except its the prosemirror json.

---

- Done? \/
- Cursor opacity fading (0-30s full, 30-60s fade, 60s-5m at 0.3, 5m+ hidden)
- Strobing slows as cursors fade (1s → 3s cycle), stops at minimum
- Strobing stops on WebSocket disconnect
- Hover on cursor line OR label → 100% opacity, no strobing
- Hover brings cursor group to top z-index
- 1s delay before returning to previous state after hover
- Horizontal tooltip stacking (most recent left, inactive right)
- User's cursor position triggers full visibility for overlapping cursors
- Reconnect cleanup (1s timer, fresh marking, cancel on disconnect)
- Debug logging enabled by default forw serve/run/repl

- websocket sends empty string message instead of ping was this deleted
- grey out offline isn't working
- ensure binary messagepack

---

- nym transport ?
- veilid transport ?
- veilid alternative network ?
- tor ?
- mycelium ?
- zerotier ? no because licenses?
- yggdrasil ?
- tinc ?
- headscale/tailscale ?
- implement iroh in clan? (unrelated to this project, really)

---

- technically unrelated to this project
  - bunx oh-my-openagent install
  - bunx oh-my-opencode install --no-tui --claude=no --openai=no --gemini=no --copilot=yes --opencode-go=no --opencode-zen=no --zai-coding-plan=no
  - https://github.com/kdcokenny/ocx
  - ocx add kdco/workspace --from https://registry.kdco.dev
  - ocx add kdco/worktree --from https://registry.kdco.dev
  - ocx add kdco/background-agents --from https://registry.kdco.dev
  - ocx add kdco/notify --from https://registry.kdco.dev
  - https://github.com/NeuralNomadsAI/CodeNomad
  - https://github.com/morapelker/hive
  - bun add -g btca opencode-ai
  - btca connect --provider opencode --model claude-haiku-4-5

---

- remove chromium/firefox from nix-common since playwright handles, still include them in the nixos config
- set zen browser as default browser in nixos config

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

---

## Long term: group encryption and authorization

- Group key agreement so a world's documents and envelopes can be encrypted for its current members only. Candidates: keyhive's BeeKEM (pre-alpha, revisit when stable) or p2panda-encryption. Borrow ideas, do not depend on pre-stable crates without review.
- Membership changes rotate keys. A removed member must not read content written after removal. Decide whether past content is re-keyed.
- Encryption at rest for world stores and the outbox, and in transit between servers over iroh.
- Authorization as signed, attenuable, revocable grants (keyhive-style). This builds on the removal and home-server artifacts. Strong-remove concurrency rules come from p2panda-auth.
- Audit the crypto choices before any release. No hand-rolled ciphersuites.
