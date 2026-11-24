# Tutorial: edit a document together in the web UI

This tutorial runs one node with the web UI, creates a document, and edits it from two browser windows at once. Both windows connect to the same collaborative editing session, see each other's cursors, and converge on one document. One machine is enough: the second window can be a second browser profile or a private window. A second person on the same network works too, with the bind and token changes noted in Step 1.

The web UI is documented in [The web UI: pages and HTTP API](../reference/web-ui.md), and its wire protocol in [WebSocket protocols](../reference/websockets.md). The editor stores documents in the same blob store as the command line, so a file written here is visible to `id list` and `id cat` afterwards — the path between the two surfaces is [Tutorial: store, share and fetch a file between two nodes](share-a-file.md) in reverse.

## Step 1: serve with the web UI

```sh
id --new collab serve --web
```

`--new collab` creates the node's data directory `.iroh/collab/`; on later runs reuse it with `--data-dir .iroh/collab`. The flag `--web` starts the HTTP web UI. Its literal defaults: `--port` is 3000 and `--bind` is `127.0.0.1`, so the UI is at `http://127.0.0.1:3000` and reachable only from this machine. Startup prints a warning when a web listener is bound without a gate token. For this tutorial on one machine, the loopback default is the correct posture and the warning is expected.

To let a second person on another machine in, both changes are needed: bind a non-loopback address with `--bind` or a repeatable `--web-listen <IP:PORT>`, and set a gate token with `--web-token` (environment variable name `ID_WEB_TOKEN`). The token is a secret value and is never printed in this book — only its name is. A visitor supplies it once; the server accepts it as the cookie named `id_token`, as an `Authorization: Bearer` header, or as a `?token=` query parameter, which sets the cookie and redirects to the clean path. The flag `--web-cookie-secure` marks the cookies `Secure` for deployments behind a TLS-terminating proxy.

One build requirement: the binary must have the `web` feature. The Nix `id-web` package (the default package) is built with `web + sandbox + ssh + world`; a plain default Cargo build is not — its defaults are `sandbox` and `world` only.

## Step 2: create the document

Open `http://127.0.0.1:3000` in the first browser window. The home page lists worlds and globally tagged documents. Go to the file browser at `/files` and create a new file named `meeting-notes.md`. Creation posts to `POST /api/new`; saving posts editor content to `POST /api/save`. The `.md` extension selects the Markdown content mode, one of the four editable modes: Rich (`.pm.json` ProseMirror JSON), Markdown (`.md` and `.markdown`), Plain (`.txt`), and Raw (code and config extensions, and the default for unknown extensions). Editable modes get the editing toolbar in Rich, Markdown, and Plain modes.

Type two lines of toy content:

```text
Meeting notes
First line written by window one.
```

The editor auto-saves. The file is now a named blob in the node's store.

## Step 3: open the same document in a second window

In the second window, open the editor URL for the same file: `http://127.0.0.1:3000/edit/meeting-notes.md`. The route is `GET /edit/{*name}` — editing is by name. (The related routes are `GET /view/{*name}` for a read-only view, `GET /hash/{hash}` to redirect from a content hash to its named file, and `GET /blob/{hash}` for raw bytes.)

Both windows now hold a WebSocket connection to `/ws/collab/{doc_id}`, where the document id identifies this document's editing session. The protocol is binary MessagePack, with messages as arrays `[tag, …fields]`. On connect, the server sends Init, `[0, version, doc, mode]`; the client answers Auth, `[8, token]`, when a gate token is in force, and receives AuthOk, `[9, client_id, name, token?]`. Keep both windows open and side by side.

## Step 4: edit together

Type a line in window one. The edit leaves as Steps, `[1, version, steps, clientID]`; the server acknowledges it to window one with Ack, `[3, version]`, and broadcasts it to window two as Update, `[2, steps, clientIDs]`, where the editor applies it. Move the cursor in either window: Cursor frames, `[4, clientID, head, anchor, name?, idleSecs?]`, flow in both directions, and each window renders the other window's named cursor. When a client disconnects, the server broadcasts CursorRemove, `[6, clientID]`, and removes the cursor state 5 minutes after disconnect.

Edit the same line from both windows in turn. Both editors converge on the same text: the server orders the steps, and each client applies the server's Update stream. When a save produces a new stored version, the server sends NewVersion, `[7, hash, name]` to the connected clients.

Two protocol details worth knowing while you watch it work. A named editing session can publish a durable note-session checkpoint: the client sends Checkpoint, `[10, text]`, and the server answers Checkpointed, `[11, head]`, where the head is the hex of the new note head revision; filename sessions refuse Checkpoint. And the connection is kept alive on a fixed cadence: the server pings every 30 seconds of inactivity, every 2nd ping cycle sends an empty text message instead (the cursor-refresh ping), and a connection with no pong or message for 30 minutes is closed. Document state is cleaned up 1 hour after the last client leaves.

## Step 5: confirm the result outside the browser

Back in a terminal, read the document through the command line:

```sh
id --data-dir .iroh/collab list
id --data-dir .iroh/collab cat meeting-notes.md
```

The list shows `meeting-notes.md`, and `cat` prints the final merged text, including the lines written from both windows. The web UI and the CLI are two surfaces over one store; nothing was exported or converted.

## What success looks like

- Both windows show the same document text, updating live as either side types.
- Each window shows the other window's cursor with its display name.
- `id cat meeting-notes.md` prints the merged final text from the node's store.
- The document survives a restart: serve again with `--data-dir .iroh/collab serve --web` and reopen `/edit/meeting-notes.md` to continue editing it.

## One failure shape

Start the web UI on a binary built without the `web` feature — a plain default Cargo build, whose features are `sandbox` and `world`:

```sh
id serve --web
```

Startup fails before anything is served, with the error `the web interface was requested (--web/--web-listen/--port/--bind), but this id binary was built without the web feature; rebuild with --features web to serve it`, and exit code 1. The same guard fires when `--web-listen`, or an explicitly set `--port` or `--bind`, is given without `--web`. Rebuild with `--features web` or use the Nix `id-web` package, and re-run Step 1.
