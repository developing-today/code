# Future Plans

---

## **Warning For Agents & Onlookers**
> This is for future implementation.
>
> It's fine to read this file but don't make any significant decisions based on anything here.
>
> Anything here is just idle planning and is subject to change.

---

- cursors that are inactive and losing opacity should slowly stop strobing. once they hit minimum opacity until timeout they reactivating them should enable full strobing.
- ensure cursors become totally visible when a client hovers on them.
- i'm not totally sure the cursors are actually reducing opacity. review the code make sure it seems right.
- add client logs for when a cursor first gets marked as inactive, when it reaches minimum, when it is removed
- add server logs for all messages sent and received, client disconnects, inactivity, page loads, etc.

- validate that inactive cursor is based on websocket inactive / lack of client pong or other activity. cursor being idle in one place shouldn't cause it to be hidden if the client is still connected. a connected clients cursor should always be visible on other clients.
- add client logging for hover/unhover

- make a toggle in the ui for modifying cursor visibility, full minimal hidden (full is now, minimal is just the cursors themselves are shown, hidden doesn't show cursor info at all even though it is still tracked internally.). cursors include their highlight state, so minimal just removes the tooltip.
- if a minimal cursor is hovered show the tooltip for at least 1 second, if the tooltip or cursor aren't hovered then re-hide after one second.

- if a user clicks a cursor tooltip it should get slightly larger for at least 1 second and return to previous state if not hovered after 1 second. (so if a minimal is hovered to tooltip and then clicked to big, and then the mouse doesnt hover anymore, it should revert to tooltip after 1 second and then back to minimal after 1 more second)

- a big cursor tooltip should have a '-' button to hide the cursor tooltip immediately as just a cursor and if the tooltip is already hidden by default (so the tooltip was a cursor only, someone hovered to get the tooltip shown, then clicked the shown tooltip), a plus button should be made in the same place minus was instead to allow that cursor tooltip to be shown by default. so 'full' should have a list of minimized and 'minimal' should have a list of restored.

- a big cursor tooltip should have a pin/unpin button which doesn't allow the big tooltip from hiding. if a cursor was hidden and someone pins it it is still hidden so on-unpin it goes back to hover timeout and if timed out goes back to a cursor. pinned cursors should be visible regardless of cursor visibility toggle, so one can pin a cursor in full or minimal and that cursor will be visible in hidden mode.

---

- the website header/footer sometimes are doubling themselves, please fix this bug. it seems to happen most often when clicking on a page like 'settings' or something. or maybe if a page is still open when a server is redeployed?

- user's client id should be shown on the settings page, right now it just shows 0's. each new tab can still generate a new random id, but changing pages from within the website shouldn't generate a new id. later we may implement stronger persistence options, but right now as long as clicking around uses the same id that's good. opening a page in a new tab can behave either way i don't care so long as a fresh reload from a new tab autogenerates. does server make the id right now? if so continue doing that, we dont have an authorize mechanism. a user shouldn't be able to spoof another user's id because it's server controlled. maybe let the websocket persist with page transitions? if this isn't possible with default js/browser mechanics, maybe htmx can help load new page / update then update the url location, without actually 'loading' a new page.
- allow setting 'name' in the settings and push that name to the server, all other clients should get a new cursor message with the same/current cursor state and the name set. names don't need to be sent every time. they should be sent when a user sets their name or renames themselves and they should be sent when a new client joins a page because they dont have any cursor info yet. the settings page should show their name under the id but above the theme selection.
- the themes on the settings page the names of them are on top of the icons. ensure the names are below their respective icons with proper spaces no overlap, the color of each name should correspond to the color used in the theme-- not the color of the current theme.

---

- RUST_LOG env var is fine, but support setting log level by cmdline input. link to this document in appropriate places https://docs.rs/env_logger/latest/env_logger/
- use structured logging everywhere. in each server and web client log send a json log message. update the slog messages to have appropriate context and properties for the given log message, configure trace ids and use contexts or properties to mark what the log is from. include a nanosecond rfc timestamp including timezone, default to host timezone fallback utc. allow overriding time zone.
- enable being able to log to a file, enable being able to change the log file location by command line, consider if it's possible to send different log levels to different places. consider if stdout can show info while one log file has up to debug logs and another has up to info logs, for instance.

---

- build variants can that file be made by the cargo build command somehow? instead of in justfile / build.sh it's an additional artifact of the build itself?
- add this to the just file and nix flake, to allow formatting the justfile automatically, use --check in the ci version and fmt in fmt which should be used in fix https://just.systems/man/en/formatting-and-dumping-justfiles.html
- justfile cargo test maybe doesn't need all feature for some of the subcomponent tests. verify what this does, i guess if lib has web features it should be included. don't make new test types without checking.
- fix so meta attribute is on all nix flake sections that need it.
- just ci is running multiple tests in a row, i see it repeated the 237 test blocks and it may be repeating more tests in some of the smaller groups like the 54/14 tests. does the just command need changes?
- nix flake check fails because clippy has to build and tries to download files. find a way to ensure that no network access is needed. if just ci needs to have different commands then ensure just check still runs all of them. if you can fix it by updating the build or something then do that.
```
❯ nix flake check
warning: Git tree '/home/user/code' is dirty
warning: app 'apps.x86_64-linux.default' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.just' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.fmt-check' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.lint' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.test' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.test-unit' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.test-int' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.test-web' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.doc' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.check' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.ci' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.fix' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.fmt' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.lint-fix' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.test-verbose' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.doc-open' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.coverage' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.coverage-open' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.coverage-summary' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.build' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.build-lib' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.build-force' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.build-lib-force' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.release' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.release-lib' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.release-force' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.release-lib-force' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.web' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.web-force' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.web-dev' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.run' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.repl' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.serve' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.serve-lib' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.build-check' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.build-check-serve' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.build-check-serve-lib' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.build-serve' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.build-serve-lib' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.watch' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.watch-test' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.watch-lint' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.outdated' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.audit' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.machete' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.update' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.tree' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.clean' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.loc' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.check-all' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.test-lib' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.build-web' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.build-release' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.build-lib-release' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.web-build' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.web-typecheck' lacks attribute 'meta'
warning: app 'apps.x86_64-linux.watch-build' lacks attribute 'meta'
error: Cannot build '/nix/store/mk9mmd71fv2qzajazsm17d2s0ixhhwv5-id-lint.drv'.
       Reason: builder failed with exit code 101.
       Output paths:
         /nix/store/k2gkxav20k5xhkmyjqd2qhh34gwc4hrh-id-lint
       Last 36 log lines:
       > Running phase: unpackPhase
       > unpacking source archive /nix/store/wkkd10lzz2j162sl478xqjksbi489rqb-id
       > source root is id
       > Running phase: patchPhase
       > Running phase: updateAutotoolsGnuConfigScriptsPhase
       > Running phase: configurePhase
       > no configure script, doing nothing
       > Running phase: buildPhase
       > cargo clippy --all-targets --all-features
       >     Updating crates.io index
       >     Updating git repository `https://github.com/rustonbsd/distributed-topic-tracker`
       > warning: spurious network error (3 tries remaining): failed to resolve address for github.com: Temporary failure in name resolution; class=Net (12)
       > warning: spurious network error (2 tries remaining): failed to resolve address for github.com: Temporary failure in name resolution; class=Net (12)
       > warning: spurious network error (1 try remaining): failed to resolve address for github.com: Temporary failure in name resolution; class=Net (12)
       > error: failed to get `distributed-topic-tracker` as a dependency of package `id v0.1.0 (/build/id)`
       >
       > Caused by:
       >   failed to load source for dependency `distributed-topic-tracker`
       >
       > Caused by:
       >   Unable to update https://github.com/rustonbsd/distributed-topic-tracker#c9bb5e67
       >
       > Caused by:
       >   failed to clone into: /build/tmp.3BTYPsNgtD/.cargo/git/db/distributed-topic-tracker-0342203d665c9e54
       >
       > Caused by:
       >   revision c9bb5e670f69c83c68c6864c0f288795d35954df not found
       >
       > Caused by:
       >   network failure seems to have happened
       >   if a proxy or similar is necessary `net.git-fetch-with-cli` may help here
       >   https://doc.rust-lang.org/cargo/reference/config.html#netgit-fetch-with-cli
       >
       > Caused by:
       >   failed to resolve address for github.com: Temporary failure in name resolution; class=Net (12)
       > error: Recipe `lint` failed on line 47 with exit code 101
       For full logs, run:
         nix log /nix/store/mk9mmd71fv2qzajazsm17d2s0ixhhwv5-id-lint.drv
error: build of '/nix/store/5s64nhc9q6q3kfcc2g7qhhdkpipqh3q4-id-doc.drv', '/nix/store/i3gllpzwkhcplf7hdjhqyv9kklilaywc-id-test.drv', '/nix/store/j822h0x7fivhnbkj1xkcwxijjs5fdc5d-id-ci.drv', '/nix/store/mk9mmd71fv2qzajazsm17d2s0ixhhwv5-id-lint.drv', '/nix/store/qz8s4cmg7clwjp431qnsh7c104n2r3s3-id-test-web.drv', '/nix/store/s64a8ncnk42zd6hy1nfw8nl55b0knp4q-id-test-unit.drv', '/nix/store/ygwwkj3ammddh9qc0bqn8c0w9dp6z4nd-id-test-int.drv' failed
```

---

- currently prosemirror is sending json inside a binary compacted message, how efficient is this? would there be any advantage to making this actually binary or compressing/uncompressing it? consider client vs server calculation cost, speed/latency, wire/memory size

---

- ensure website can collaboratively edit files with multiple clients connected to the same server, and that changes are reflected in real time across all clients. each document opened in the website should have it's own collaboration session. handle conflicts gracefully if multiple clients edit the same file at the same time. figure out how best to sync the state of the file back to the store, we don't want a new file for every character typed. maybe to startt we can just have a "save" button that syncs the current state of the file to the store, and then we can look into more real-time syncing options later, as well as some method of garbage collecting old file versions.
- ensure website looks nice and has whatever appropriate testing is needed for the features and for a webserver/website.

---

- ensure website has 1:1 capability with cli (can run equivalent of each command/flag except from a native browser ui)
- implement access to the repl from the website
- implement a method to switch the server one is connected to from the website

---

- implement a method to send files from one server to another in cli/repl and website
- implement a method for each client to give petnames to any other clients/servers/nodes they interact with, and have those petnames be used in the cli/repl and website instead of the actual names. should be able to assign any arbitrary node id but ideally have a convienient ways to assign petnames to nodes that are interacted with in the repl/website.

---

- can we add tags to files/nodes and have those tags be searchable in the cli/repl/website? each client/node that added the tag should be linked to the tag so we can show that information in the cli/repl/website and use it for searching/organizing. if 2 clients add the same tag to a node/file, that should be reflected in the cli/repl/website and show that both clients added that tag, in the order they added it with timestamps. (get all files from x node that have the word 'yz' in them etc should be doable, not reinventing sql just allowing extended search/metadata.)
- can each uploaded file be linked to the node/time that uploaded it? (maybe use tags for this? or whatever is best) implement at least a rudimentary way to show this information in the cli/repl/website, and use it for searching/organizing files. ideally we could also show this information in the file listing in the cli/repl/website, and have a way to sort/filter by upload time/node.
- can tags/petnames be able to be either local to the client or shared across clients, and can we have a way to specify which when creating/editing a tag/petname? some way to organize/review existing petnames/tags would be good too.
- allow each node to self-publish information about themselves, their preferred name, maybe just helpers to add public tags to their own node id and then update the various interfaces to be able to poll and pull that info. (priority would be something like "clients private alias for some node, clients public alias, the other node's public alias for themselves, any other public aliases the client/server are aware of, 

---

- make a tui using ratatui or whatever the highest performance tui rust crate is. aim for high performance, it should work locally without running other servers, or connect to the local server if it's running or connect to a remote server. use iroh for network communication not ssh, consider the most efficient way to handle this so that it is very performant. we want high fps, ability to make complex graphs, possibly transmit kitty image protocol images, coloring blocks, all the tui things you might want from something like ratatui, but ideally you wouldn't be sending all the terminal inputs from the server to the client. there should be a way to send a custom protocol in an iroh postcard or whatever, where you say what you want to do provide the new bytes, and then the client handles. like 'heres the bytes for the image, put the image in the screen at 64x 16y on the screen and let the image be 256*256' and then the client can display the picture without the server needing to actually move the cursor, delete/redraw in the terminal, etc. server could say 'draw the level map at <blob> across the entire screen' and then the client would handle getting the blob itself. you wouldn't want a second round trip if they don't have it cached locally, so some thought would need to be put there. the tui should cover things like the website, except native access without upload boxes and with full control of the box. it should be a tui program, doesn't need to be ssh--- someone can ssh into the server and run the tui. maybe 3 panels, room/object selector/search/find/pin-favorites || the interactive room/document or other meta configuration or search pages || a chatroom for a given room or topic

---

- make a native gui using something like tauri, egui, tauri-egui, leptos, dioxus, iced, bevy, etc. try to remember we may later embed bevy project into the native app or embed the native app into bevy. it should do what the tui does, but faster and better.

---

- rust/js linters, clippy with all runs, run rustfmt, etc. (youtube video that mentioned what to run? there was another in addition to clippy..)


---

- update all nix flake check commands to have -L nix flake check -L
- just test-all which runs all tests including e2e. make check all too
- enable running multiple servers on different ports if possible.
- ensure integration tests can run in nix flake check, ensure nix flake check can run integration/web non-unit tests. i believe this would involve  making a virtual machine and wait for open port. maybe you need to make a nixos service for 'id' ?
  - https://blakesmith.me/2024/03/11/how-to-run-nixos-tests-flake-edition.html
  - https://nixcademy.com/posts/nixos-integration-test-on-github/
  - https://github.com/tfc/nixos-integration-test-example
  - https://blog.thalheim.io/2023/01/08/how-to-use-nixos-testing-framework-with-flakes/
- build a check id github workflow which uses magic nix cache and runs nix flake check against the id flake

---

- also add bun2nix in the main flake.nix ensure i can run bun2nix without needing to use bunx. (package install should be done in environment/default.nix like other inputs)
- ensure bun2nix is available from nix-common, as a command, and add a just command just bun2nix which automatically builds the bun.nix file. alias lockfiles to call bun2nix. rename just fmt to cargo-fmt with just fmt alis to call only just cargo-fmt. rename lint to clippy-lint and alias lint to clippy-lint. fix should call lockfiles fmt lint
- update deploy.sh to run bun2nix any time the bun lockfile changes. ensure flake.nix has appropriate apps to just bun2nix like all the other just commands. can bun2nix also be a 'formatter' that is ran? can you have all formatters run with nix fmt r is it just one? if multiple allowed then make them different.
- install formatting/linting for bun, make web-fmt web-lint make fmt call cargo-fmt web-fmt. make lint call web-lint cargo-lint. if there is an autofix for the linter then web-lint-fix and use that in lint-fix. make web-fmt-check and call that in ci: along with web-lint. if there are multiple useful formatters or linters, choose the fastest modern formatter and similar for linter though you could allow multiple linters if they don't conflict.
- ci: should also build debug&&release versions of the app, just build and just release.
- fix all new web lints.
- if deploy.sh is running any other commands that exist in `justfile` then update deploy.sh to use those just commands. ensure 1:1 between justfile and flake.nix is maintained.
  - calls to cargo bun bun2nix etc. should be handled by justfile. deploy.sh should handle setting correct vars/toggles when required. minimal changes to justfile commands to allow needed env/flag is allowed here, but if you can just set env vars without needing other changes then do that. you shouldn't use nix in this script, only just bun2nix to ensure nix _should_ work.
- nix fmt and nix flake check should both work. you may need to ensure that bun2nix is integrated to handle this.

---

- make a new plan file, it should cover each of these todos. make a todo covering each of these. reorder and rephrase your todos as needed, but put this entire prompt in a section of the plan so one can refer back to the original requests. your plan should cover architecture changes, before/after, design/intent, exact steps to be taken and the expected end result. your plan file should have your final todo list too in it's own section. use `just` commands to manage build/test. just check for all testing. just kill-serve will build and then deploy the server, at the very end `nix flake check` should be ran and it should pass just like `just check` and if it doesn't then fix.
- repl and cli should be 1:1, ensure any diff in one is copied into the other, such as the aliases etc.
- ensure docs keys can be bytes/binary allowing structured data. either key or value can be bytes and not just string.  review the original tags v2 plan at .opencode/plans/2026-03-24-tags-v2-iroh-docs.md
- ensure tag keys and values can be any length and any kind of bytes, but try to parse as utf8. cutoff in the ui/etc after a certain length. if it doesn't parse as utf8 then exclude as binary without a flag.
- allow searching keys and values in the ui. strings ending with : mean keys, starting with : mean values key:value means that specific key. allow quotes to do regular search so ":key" looks for any file/key/value that is literally ":key". "key:":":value" looks for a key named "key:" with a value ":value". a file uploaded should automatically tag name,file,path tags to the file. all name/file/path keys should show as the name in the ui, but they should dedupe if they have the exact same hash, tag values, name of the file. make a migrate flag that automatically adds the name/file/path labels for all existing files which don't have them. 
- when you are done add appropriate unit/integration/playwright tests as-needed.
- when you are done review all .md files across the repo and ensure they are up-to-date.
- when you are done, nix flake check should pass 100%. fix any errors even if you dont think you made them.
- when you are done, update all docstrings, comments, help text for all functions and objects in rust and typescript.
- 
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
