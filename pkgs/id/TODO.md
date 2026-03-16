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
