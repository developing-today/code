# WebSocket protocols

Source: https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-web/src/collab.rs, https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-web/src/tags_ws.rs, https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-web/src/world_ws.rs, https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-worlds/src/world_session/types.rs

`id serve --web` exposes three WebSocket protocols. Their routes are listed in [The web UI: pages and HTTP API](web-ui.md); this chapter is the wire reference for each.

| Socket | Route | Encoding | Direction |
|---|---|---|---|
| Collab | `/ws/collab/{doc_id}` | Binary MessagePack | Both |
| Tags | `/ws/tags` | JSON text | Server to client only |
| World bridge | `/ws/world` (default; from `[interfaces.web]`) | JSON text | Both |

## Collab: collaborative editing

Implementation: `crates/id-web/src/collab.rs`. Frames are MessagePack arrays of the form `[tag, …fields]`, encoded and decoded with `rmp_serde`. The message type is `CollabMessage`.

| Tag | Name | Direction | Fields |
|---|---|---|---|
| 0 | Init | Server → client | `[0, version, doc, mode]` |
| 1 | Steps | Client → server | `[1, version, steps, clientID]` |
| 2 | Update | Server → client | `[2, steps, clientIDs]` |
| 3 | Ack | Server → client | `[3, version]` |
| 4 | Cursor | Both | `[4, clientID, head, anchor, name?, idleSecs?]` |
| 5 | Error | Server → client | `[5, error]` |
| 6 | CursorRemove | Server → client | `[6, clientID]` |
| 7 | NewVersion | Server → client | `[7, hash, name]` |
| 8 | Auth | Client → server | `[8, token]` — the first message after connect |
| 9 | AuthOk | Server → client | `[9, client_id, name, token?]` — the `token` field carries a refreshed token when one is issued |
| 10 | Checkpoint | Client → server | `[10, text]` — publish a durable note-session checkpoint. Filename sessions refuse this message. |
| 11 | Checkpointed | Server → client | `[11, head]` — `head` is the hex of the new note head revision |
| — | Empty text `""` | Both | Cursor-refresh ping/pong for throttled inactive tabs; see the cadence below |

Compatibility: a legacy Init frame without the `mode` field still decodes; `mode` defaults to `raw`.

### The route parameter (WEB-6)

The route is `/ws/collab/{doc_id}`. The `web/README` connection flow names the parameter `{file_path}`. That is disagreement WEB-6: the registered route, and `WEB.md`'s own table, both use `{doc_id}`.

### The documented tag table is six tags behind (WEB-5)

The `web/README` wire table lists tags 0–5 only, and gives Init as `[0, version, doc]`. The code defines tags 0–11, and Init is `[0, version, doc, mode]`. CursorRemove (6), NewVersion (7), Auth (8), AuthOk (9), Checkpoint (10) and Checkpointed (11) are absent from that README. The `collab.rs` module documentation is complete; the README is the stale copy. That is disagreement WEB-5.

### Keepalive, timeouts and cleanup

| Constant | Value | Behaviour |
|---|---|---|
| `PING_INTERVAL` | 30 s of inactivity | A Ping is sent every 30 s idle cycle. |
| `EMPTY_TEXT_MESSAGE_EVERY_N_PINGS` | `2` | Every 2nd ping cycle sends the empty text message `""` **instead of** a Ping frame. |
| `PONG_TIMEOUT` | 30 min | The connection is closed after 30 min without a pong or message. |
| Cursor removal | 5 min after disconnect | A disconnected client's cursor is removed. |
| Document cleanup | 1 h after the last client leaves | The document's in-memory collab state is cleaned up. |

### The empty-text cadence (WEB-8)

The `web/README` timeout section states that the server sends empty text messages *instead of* Ping frames. The code sends both: a Ping frame on each 30 s idle cycle, with every 2nd cycle's Ping replaced by the empty text message. A client must therefore handle Ping frames and empty text frames on the same socket. That is disagreement WEB-8; the cadence in the table above is the code behaviour.

### Worked example

Toy data: a document `notes.pm.json`, one client, one edit.

```text
C→S  [8, "TOKEN"]                          Auth, first message after connect
S→C  [9, 41, "Ada", "TOKEN2"]              AuthOk: client_id 41, refreshed token
S→C  [0, 7, "notes.pm.json", "rich"]       Init: version 7, mode rich
C→S  [1, 7, <steps>, 41]                   Steps based on version 7
S→C  [3, 8]                                Ack: now at version 8
S→C  [2, <steps>, [42]]                    Update: steps from client 42
C→S  [4, 41, 120, 118, "Ada", 0]           Cursor at head 120, anchor 118
S→C  [7, "0123abcd…", "notes.pm.json"]     NewVersion after a save
C→S  [10, "draft text"]                    Checkpoint (note sessions only)
S→C  [11, "beef…"]                         Checkpointed: new head revision hex
```

A filename-based session that receives tag 10 refuses it; checkpoints exist only for note sessions.

## Tags: tag-change broadcast

Route: `/ws/tags`. Implementation: `crates/id-web/src/tags_ws.rs`; the event type is `TagEvent` in `crates/id-store/src/tags.rs`. Frames are JSON text. The socket is server → client only: no inbound messages are expected.

`TagEvent` is serialised with `#[serde(tag = "type")]` and lowercase renames. The four kinds are:

| `type` | Fields | Meaning |
|---|---|---|
| `set` | `ns`, `subject`, `key`, `value?` | A tag value was set. |
| `del` | `ns`, `subject`, `key`, `value?` | A tag value was deleted; `value` absent means the key's values were deleted. |
| `del_all` | `ns`, `subject` | All tags for the subject were deleted. |
| `transfer` | `ns`, `from`, `to`, `count` | `count` tags moved from subject `from` to subject `to`. |

Namespaces: `ns` is `global` (the default), `node`, or a custom paired (α/Ω) namespace. Tag values are arbitrary bytes on the store side; the REST surface in [The web UI: pages and HTTP API](web-ui.md) carries binary values as `value_b64`.

### The documented field names do not parse (WEB-7)

Both `WEB.md` and `web/README` show the event types capitalised — `{"type":"Set"|"Del"|"DelAll"|"Transfer", …}` — and name the transfer fields `from_subject` and `to_subject`. The code serialises lowercase type names (`set`, `del`, `del_all`, `transfer`), and the transfer fields are `from`, `to` and `count`. There are no `from_subject` or `to_subject` fields. A client written to the documents cannot parse real frames. That is disagreement WEB-7; the table above is the wire format.

### Worked example

Toy data: a file named `report.txt` in the `global` namespace.

```json
{"type":"set","ns":"global","subject":"report.txt","key":"status","value":"draft"}
{"type":"set","ns":"global","subject":"report.txt","key":"status","value":"reviewed"}
{"type":"del","ns":"global","subject":"report.txt","key":"status","value":"draft"}
{"type":"transfer","ns":"global","from":"report.txt","to":"report-final.txt","count":3}
{"type":"del_all","ns":"global","subject":"report-final.txt"}
```

## World bridge: world session frames

Socket: `/ws/world` by default. The path is the app's `[interfaces.web]` `socket` value; see [Configuration and environment](configuration.md). The adapter is `crates/id-web/src/world_ws.rs`. The protocol itself is owned by `id_worlds::world_session` and is shared with the Iroh transport, so the same frames travel over both.

Rules that apply to every frame:

- Frames are JSON text.
- Each frame is bounded by `MAX_FRAME_BYTES`.
- The first frame must arrive within the 10 s join timeout (`SessionConfig::default`). A client that connects and sends nothing for 10 s is dropped before it joins.
- Client frames are the `ClientFrame` type, server frames the `ServerFrame` type, both in `crates/id-worlds/src/world_session/types.rs`, serialised with the serde tag `type` in snake_case.

### Client frames: 43 kinds

| Kind | Notes |
|---|---|
| `join` | Fields: `capability`, optional `after`, `screen_patches`, `screen_msgpack`, `resume_token`, `agent_profile`, `device_input`. |
| `info` | Session information request. |
| `metrics` | Metrics request. |
| `records` | Records request. |
| `records_ticket` | Records ticket request. |
| `documents` | Document list request. |
| `document` | Single document request. |
| `download_module` | Module download. |
| `download_module_admin` | Module download, admin form. |
| `event_history_admin` | Event history, admin form. |
| `invite` | Carries an `admin_token`. |
| `directory` | Public directory request; no token. |
| `list_worlds` | List worlds on the host. |
| `create_world` | Create a world. |
| `set_isolation` | Set a world's isolation state. |
| `set_cross_world_write` | Set cross-world write. |
| `force_cross_world_write` | Force cross-world write on. |
| `set_file_scope` | Set file scope. |
| `compile` | Compile a module. |
| `update_caps` | Update capabilities. |
| `set_caps_ceiling` | Set the capability ceiling. |
| `set_schedule` | Set a schedule. |
| `cancel_schedule` | Cancel a schedule. |
| `world_state` | World state request. |
| `set_execution_profile` | Set the execution profile. |
| `execution_profile_info` | Execution profile information request. |
| `caps` | Capability query. |
| `chat` | Chat post. May carry `screen_revision`, the stale-decision token checked by the policy layer in [The MCP bridge](mcp.md). |
| `form_result` | A submitted form result. |
| `attenuate` | Attenuate a capability. |
| `delegate_to_friend` | Delegate to a friend. |
| `input` | Program input; the payload travels as `data_hex`. |
| `resize` | Terminal/surface resize. |
| `paste` | Paste input. |
| `screen_subscribe` | Subscribe to a screen region and/or to event classes. |
| `pointer` | Pointer input. |
| `hover` | Hover input. |
| `install_begin` | Begin a module install upload. |
| `install_chunk` | One install chunk. |
| `install_end` | Finish a module install upload. |
| `revoke` | Revoke a capability. |
| `provider_offer` | Provider offer. |
| `provider_reply` | Provider reply. |

### Server frames: 35 kinds

| Kind | Notes |
|---|---|
| `snapshot` | Full world snapshot. |
| `event` | A world event. |
| `provider_request` | A provider request to the client. |
| `form_result_status` | Status of a form result. |
| `view` | Screen view: `data`/`surface`/`patch`, plus `view_revision` and `resume_token`. |
| `semantic_diff` | A semantic diff of the screen state. |
| `screen_subscription` | A screen subscription delivery. |
| `module_info` | Module information. |
| `metrics` | Metrics reply. |
| `records` | Records reply. |
| `records_ticket` | Records ticket reply. |
| `documents` | Documents reply. |
| `document` | Document reply. |
| `module_begin` | Begin a module download. |
| `module_chunk` | One module chunk. |
| `module_end` | End a module download. |
| `event_history_begin` | Begin an event history download. |
| `event_history_event` | One event history entry. |
| `invite` | Invite reply. |
| `worlds` | World list reply. |
| `world_created` | A world was created. |
| `isolation` | Isolation state. |
| `policy` | Policy state. |
| `caps` | Capability reply. |
| `execution_profile` | Execution profile reply. |
| `caps_ceiling` | Capability ceiling reply. |
| `schedules` | Schedules reply. |
| `world_state` | World state reply. |
| `compiled` | A compile finished. |
| `upload_ready` | The host is ready for an install upload. |
| `module_installed` | A module was installed. |
| `revoked` | A capability was revoked. |
| `program_changed` | The program changed. |
| `error` | An error. May carry a stable `code`, for example `actor_denied`. |
| `input_refused` | An `input` frame was refused. |

### Contract revisions

Two contract revisions are referenced in the code:

- **Revision 11** — agent profiles and semantic diffs. The subscribable event classes are `chat_posted`, `input_applied`, `object_changed`, `system_event` and `program_changed`. Two further change kinds exist and are not subscribable: `region_changed` and `notification`.
- **Revision 12** — negotiated binary MessagePack screen patches. This revision applies only on byte-capable transports.

### Worked example

Toy data: a guest joining the world `lobby` with capability `CAP`.

```json
{"type":"join","capability":"CAP"}
{"type":"snapshot", "…": "full world snapshot"}
{"type":"view","view_revision":1,"resume_token":"…"}
{"type":"chat","text":"hello from a toy client"}
{"type":"event", "…": "chat event broadcast"}
{"type":"screen_subscribe","event_classes":["chat_posted","object_changed"]}
{"type":"error","code":"actor_denied"}
```

The last frame shows the failure shape: an `error` server frame whose `code` is a stable string (`actor_denied`), not free text to parse. A refused `input` frame is reported separately, as an `input_refused` frame, not as an `error`.
