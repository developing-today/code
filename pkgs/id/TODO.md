# Future Plans

---

## **Warning For Agents & Onlookers**
> This is for future implementation.
>
> It's fine to read this file but don't make any significant decisions based on anything here.
>
> Anything here is just idle planning and is subject to change.

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
