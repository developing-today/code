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

- update agent.md -- when you make a significant design or architecture decision, whether it's a feature of an existing component or a new large piece which changes how things operation, immediately memorialize the request/intent and initial plan by documenting it first-- then begin implementing. make a docs folder for this feature/design/copmonent/architecture/refactor/etc. like docs/<utc_rfc_date_time>_<kind of change>_<name of change>/<the same thing again>.md and output your findings. during the initial rollout, append modifications clarifications and other details as new sections in the file. when initial rollout is complete, if the final result is significantly different or there were many updates as you went then create a new file in the same folder with a new datetime same name, after name of change you could add one more section before .md to clarify what kind of new file this is. if you come back to this in a new session or have referred back to it and are planning to make major changes, make a new datetime document with the additional section in the name explaining what kind of revision it is. files after the initial rollout can be short notes, updates to specific parts, or complete resummarization of the current or proposed design/interface. if you aren't intending to make a change but have realized the latest summary + subsequent notes are out of date then make a new todo to provide an update datetime file at least covering the noticed differences and any understanding of intent or implication or timeline. file contents shouldn't be modified after initial creation, and after some time they should not be appended to either. they should be kept in the state they were in as a historical record and new files can be made. if you have a new large feature that will replace an old one that was documented, make a new folder based on the create date of the new feature, reference the old features folder and make a note file in the old feature folder backlinking to the new feature that subsumes the older one. (it might not be a 1:1 replacement, it could just signify a change in direction where the old feature may be deprioritized more and more over time, or not depending on what the future holds.)
- update the agents.md from top to bottom to ensure that it is tight prose that doesn't waste context. keep the clear distinctions and helpful data try not to remove any specific directions, but if things can be rewritten in a clearer way then do so. maybe consider which code examples are required for operation and simply link to the files that create other commands that may be useful, like the justfile, or link especially relevant docs and provide a copy of the basic commands of the app itself.

- make a /update command in this repo which works like /init but has specific suggestions like ensuring basic details being loaded into context have been updated such as the core features of the app, the file structure, etc. ind the update command source in the context of this message or in the anomalyco opencode repo or your own source files on disk. make a new version of /init that expects there to be an agents.md at one of the precedence levels and is targeted at ensuring its up-to-date and aligned. it shouldn't make up rules or remove rules on it's own, emphasize providing a series of questions for things it comes up with as many as it needs to provide as concise as possible agent.md while still retaining all useful details.
- is file structure helpful in context or can you just run ls yourself in a new session? if its useful then keep it so that first query response is better, but if just letting that kind of data get loaded ad-hoc is better then we can remove it to save context.

---

- make a native gui using something like tauri, egui, tauri-egui, leptos, dioxus, iced, bevy, etc. try to remember we may later embed bevy project into the native app or embed the native app into bevy. it should do what the tui does, but faster and better.

---

- rust/js linters, clippy with all runs, run rustfmt, etc. (youtube video that mentioned what to run? there was another in addition to clippy..)

---
