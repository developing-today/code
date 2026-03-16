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
- **0-30s**: Full opacity (1.0)
- **30-60s**: Fades linearly to 0.3
- **60s-5m**: Stays at 0.3 opacity  
- **5m+**: Hidden completely

The `idleSecs` field in Cursor messages is only sent when the server sends existing cursors to a newly connected client. The client backdates `lastUpdate` by `idleSecs * 1000ms` so the cursor displays at the correct opacity immediately.

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
│   ├── main.ts      # Entry point, initializes collab
│   ├── editor.ts    # ProseMirror setup, schema, menu
│   ├── collab.ts    # WebSocket client, MessagePack protocol
│   ├── cursors.ts   # Cursor/selection plugin with fade
│   └── theme.ts     # Theme switching
├── styles/
│   ├── editor.css   # ProseMirror styles, cursor tooltips
│   ├── terminal.css # Base terminal aesthetic
│   └── themes.css   # Theme CSS variables
├── dist/            # Built output (git-ignored)
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
