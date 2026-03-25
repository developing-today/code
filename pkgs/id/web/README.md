# Web Collaborative Editor

Real-time collaborative text editor for `id serve`, built with HTMX, ProseMirror, and WebSockets.

## Architecture

```
+----------------+       WebSocket       +----------------+
|   Browser(s)   | <-------------------> |   id serve     |
|  ProseMirror   |   Binary MessagePack  |   Axum/Tokio   |
+----------------+                       +----------------+
```

- **Frontend**: ProseMirror editor with `prosemirror-collab` plugin
- **Backend**: Axum WebSocket handler with in-memory document state
- **Protocol**: Binary MessagePack arrays over WebSocket

## Wire Protocol

All messages are encoded as MessagePack arrays with a type tag as the first element.

### Message Types

| Tag | Name   | Direction      | Format                                           |
|-----|--------|----------------|--------------------------------------------------|
| 0   | Init   | Server->Client | `[0, version, doc]`                              |
| 1   | Steps  | Client->Server | `[1, version, steps, clientID]`                  |
| 2   | Update | Server->Client | `[2, steps, clientIDs]`                          |
| 3   | Ack    | Server->Client | `[3, version]`                                   |
| 4   | Cursor | Bidirectional  | `[4, clientID, head, anchor, name?, idleSecs?]`  |
| 5   | Error  | Server->Client | `[5, errorMessage]`                              |
| -   | Empty  | Bidirectional  | `""` (empty text, see below)                     |

### Field Types

- `version`: u64 - Document version number
- `doc`: JSON object - ProseMirror document JSON
- `steps`: Array of JSON objects - ProseMirror steps
- `clientID`: u64 - Unique client identifier
- `clientIDs`: Array of u64 - Client IDs corresponding to steps
- `head`: u64 - Cursor head position
- `anchor`: u64 - Selection anchor position (equals head for cursor, differs for selection)
- `name`: String (optional) - Display name for cursor tooltip
- `idleSecs`: u64 (optional) - Seconds cursor has been idle, only sent on initial load

### Connection Flow

1. Client connects to `/ws/collab/{file_path}`
2. Server sends `Init` with current version and document
3. Client initializes ProseMirror with received version
4. Client sends `Steps` when user edits
5. Server broadcasts `Update` to all other clients
6. Server sends `Ack` to confirm steps applied
7. Clients exchange `Cursor` messages for selection sharing

### Empty Text Messages

WebSocket Ping control frames are handled silently by browsers and don't trigger JavaScript's `onmessage`. To allow cursor decoration refresh in inactive tabs (where `setInterval` is throttled), the server sends empty text messages (`""`) every 60 seconds instead of Ping frames.

When the client receives an empty text message, it:
1. Responds with an empty text message (as pong)
2. Refreshes cursor decorations (recalculates opacity based on `lastUpdate`)

### Cursor Opacity

Cursors fade based on inactivity to indicate staleness:
- **0-30s**: Full opacity (1.0), fast strobing (1s cycle)
- **30-60s**: Fades linearly to 0.3, strobing slows to 3s cycle
- **60s-5m**: Stays at 0.3 opacity, no strobing
- **5m+**: Hidden completely

The `idleSecs` field in Cursor messages is only sent when the server sends existing cursors to a newly connected client. The client backdates `lastUpdate` by `idleSecs * 1000ms` so the cursor displays at the correct opacity immediately.

### Cursor Hover Behavior

When a user hovers over a cursor (label or cursor line):
- Cursor immediately becomes fully visible (100% opacity)
- Strobing animation stops
- The entire cursor group at that position is brought to the top (highest z-index)
- After 1 second of not hovering, cursor returns to its previous opacity/strobing state

Hovering on the cursor line (not just the label) also triggers full visibility. This allows bringing a covered cursor group to the top by hovering on its visible line.

### Tooltip Stacking

When multiple cursors are at the same document position:
- Labels stack horizontally (not vertically)
- Order is by activity: most recently active on left, longest inactive on right
- Labels grow to the right, anchored at the cursor position
- Hovering any label in the group highlights all cursors in that group

### Cursor Line Color Cycling

When multiple cursors share the same position, the cursor line (vertical bar) cycles through all cursor colors:
- Colors cycle left-to-right through the tooltip order (most recent to oldest)
- Cycle interval: 1.5 seconds per color
- On new cursor activity: immediately shows that cursor's color, then resumes cycling
- On cursor removal: if currently showing that color, jumps to next; otherwise continues unchanged
- Cycling stops when disconnected (keeps current color)

### User Cursor Interaction

If the user's cursor (caret) is at the same position as remote cursors:
- Those remote cursors become fully visible (100% opacity, no strobing)
- This helps users see who else is editing at the same location

This allows users to inspect faded cursors without permanently changing their state.

### Connection State and Cursors

Cursor behavior changes based on WebSocket connection state:

**When disconnected:**
- All cursors keep their current opacity
- All strobing animations stop (cursors appear static)
- Reconnect cleanup is cancelled if in progress

**When reconnecting (on Init message):**
1. Client starts a 1-second cleanup timer
2. As cursor updates arrive from server, those cursors are marked "fresh"
3. After 1 second (if still connected), cursors not marked fresh are removed
4. If disconnected during this window, cleanup is cancelled and cursors remain

### Timeouts

- **Ping/Pong**: Server pings after 30s of inactivity; any message resets timer
- **WebSocket**: Server closes connection after 30m of no activity
- **Cursor**: Server removes cursor 5m after WebSocket closes
- **Document**: Server cleans up document 1h after no connections

## Themes

Three terminal-inspired themes with `#000000` black backgrounds:

- **sneak** (blue) - `#00aaff` accent
- **arch** (green) - `#00ff00` accent
- **mech** (orange) - `#ff6600` accent

## Development

```bash
# Install dependencies
bun install

# Build (outputs to dist/)
bun run build

# Watch mode
bun run dev
```

Built assets are embedded in the Rust binary via `rust-embed`.

## File Structure

```
web/
├── src/
│   ├── main.ts      # Entry point, HTMX init, file operations (rename, copy)
│   ├── editor.ts    # ProseMirror setup, schema, menu
│   ├── collab.ts    # WebSocket client, MessagePack protocol
│   ├── cursors.ts   # Cursor/selection plugin with fade
│   └── theme.ts     # Theme switching
├── styles/
│   ├── editor.css   # ProseMirror styles, cursor tooltips, viewer buttons
│   ├── terminal.css # Base terminal aesthetic, file list
│   └── themes.css   # Theme CSS variables (sneak/arch/mech)
├── dist/            # Built output (git-ignored)
├── bun.nix          # Offline npm dependency fetching for nix sandbox
└── package.json
```

## Client Implementation Notes

### MessagePack

Use `msgpackr` with `useRecords: false` for array format:

```typescript
import { Packr, Unpackr } from 'msgpackr';

const packr = new Packr({ useRecords: false });
const unpackr = new Unpackr({ useRecords: false });

// Send
ws.send(packr.pack([1, version, steps, clientID]));

// Receive
ws.onmessage = (e) => {
  const data = unpackr.unpack(new Uint8Array(e.data));
  const [tag, ...fields] = data;
};
```

### Version Synchronization

The client must initialize with the server's version:

1. Connect WebSocket, wait for `Init` message
2. Create ProseMirror editor with `collab({ version: serverVersion })`
3. Send steps with current `getVersion(state)` value

### Handling Updates

Pass ALL steps to `receiveTransaction`, including your own:

```typescript
// Server sends Update with steps and clientIDs
const tr = receiveTransaction(view.state, steps, clientIDs);
view.dispatch(tr);
```

The collab plugin automatically:
- Confirms your pending steps when they come back
- Applies remote steps from other clients
- Rebases any unconfirmed local steps

### Cursor Colors

Cursor colors are deterministically generated from clientID:

```typescript
function clientColor(id: number): string {
  const hue = (id * 137.508) % 360; // Golden angle
  return `hsl(${hue}, 70%, 50%)`;
}
```

## File Management API

The web UI provides rename and copy operations via HTTP POST endpoints. Both operations are available on all file viewer pages (editor, media viewer, binary viewer) via buttons in the page header.

### Rename (`POST /api/rename`)

```json
// Request
{ "name": "old-file.txt", "new_name": "new-file.txt", "archive": true }

// Response
{
  "name": "new-file.txt",
  "hash": "abc123...",
  "archived_original": "old-file.txt.archive.1711234567",
  "archived_replaced": "target.archive.1711234567"
}
```

- If `archive` is true, the original name is archived as `{name}.archive.{timestamp}`
- If the target name already exists, the existing file is archived as `{new_name}.archive.{timestamp}`
- All metadata tags are transferred from old name to new name

### Copy (`POST /api/copy`)

```json
// Request
{ "name": "source.txt", "new_name": "copy-of-source.txt" }

// Response
{ "name": "copy-of-source.txt", "hash": "abc123..." }
```

- Creates a new tag pointing to the same content hash
- If the target name already exists, the existing file is archived
- All metadata tags are copied from source to destination

## Tags WebSocket

The `/ws/tags` endpoint provides real-time tag change notifications. Clients receive JSON messages when tags are modified anywhere in the system.

### Event Types

```json
{"type": "Set", "ns": "global", "subject": "readme.md", "key": "author", "value": "Jane"}
{"type": "Del", "ns": "global", "subject": "readme.md", "key": "author"}
{"type": "DelAll", "ns": "global", "subject": "readme.md"}
{"type": "Transfer", "ns": "global", "from_subject": "old.md", "to_subject": "new.md"}
```

| Event    | Description                                    |
|----------|------------------------------------------------|
| `Set`    | A tag was added or updated                     |
| `Del`    | A specific tag was removed                     |
| `DelAll` | All tags were removed from a subject           |
| `Transfer` | Tags were moved from one subject to another |

The `value` field is omitted from `Set` events when the tag has no value (key-only tags). The `ns` field indicates the namespace (`global` for the default namespace).

### Tags REST API

In addition to the WebSocket stream, tags can be queried and modified via REST:

| Method   | Endpoint            | Description                          |
|----------|---------------------|--------------------------------------|
| `GET`    | `/api/tags`         | List/filter tags (query params below)|
| `GET`    | `/api/tags/search`  | Search tags with structured syntax   |
| `POST`   | `/api/tags`         | Set a tag                            |
| `DELETE` | `/api/tags`         | Delete a tag or all tags for subject |

#### GET /api/tags Query Parameters

| Parameter | Description                              |
|-----------|------------------------------------------|
| `subject` | Filter by subject (filename)             |
| `key`     | Filter by tag key                        |
| `value`   | Filter by value (requires `key`)         |
| `ns`      | Namespace (default: `global`)            |

#### GET /api/tags/search

| Parameter | Description                              |
|-----------|------------------------------------------|
| `q`       | Search query string (required)           |
| `ns`      | Namespace (default: `global`)            |

Query syntax: `key:` (key only), `:value` (value only), `key:value` (pair), `"literal"` (quoted), bare word (search all). Multiple terms are space-separated and ANDed.

#### POST /api/tags

```json
{ "subject": "readme.md", "key": "author", "value": "Jane" }
```

#### DELETE /api/tags

```json
{ "subject": "readme.md", "key": "author", "all": false }
```

Set `"all": true` to delete all tags for the subject.
