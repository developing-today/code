# Negotiated binary ScreenPatch v2 completion

This note records the design for completing the in-progress #12242 transport slice in a returning implementation session. The governing requirements remain [design_id805](../../../doc/2026-11-01_design_id805.md) §§3, 5–6 and issue [#12242](https://github.com/developing-today/code/issues/12242); this note narrows how the existing v2 transport makes binary delivery explicit and recoverable.

## Intent

Use the versioned `/ws/world/2`, `/id-world/2`, and `id-world/2` transports for ephemeral MessagePack ScreenPatch frames while preserving v1 text-JSON behavior and authorization. Binary delivery must be an explicit client opt-in, not an accidental consequence of reaching a v2 route. Preserve sequence, view-revision, event-cursor, and resume-token semantics; compression and bandwidth measurement remain out of scope.

## Decisions

- Add an optional `binary_view_patches` boolean to the join frame. A server sends binary ScreenPatch frames only when this flag and the existing `screen_patches` flag are both true and the selected transport supports binary. Missing or false means text-JSON full views/patches. The versioned route remains a transport-version choice; the join field negotiates this optional representation.
- Keep the join capability as the sole authorization check. Resume tokens remain cursors, not credentials; an absent, stale, foreign, or malformed token results in a full view.
- The browser opts in on the v2 WebSocket route, retains the most recently applied view's resume token, and includes it on reconnect. A binary decode failure requests a fresh viewport/full-view resynchronization.
- Bump the additive shell-contract revision and document the new join field plus the `version` field in the binary envelope. V1 clients remain compatible because the field is optional and omitted by them.
- Keep SSH and Iroh adapters' legacy text framing unless their negotiated adapter implements byte-safe framing; the existing v2 transport registration may advertise support only where the adapter supports it.

## Completion checks

Add Rust coverage for the join gate and codec rejection, plus TypeScript coverage for the shared binary golden vector, resume-token round-trip, and decode-error resynchronization. Run the focused Rust and browser checks and record exact outcomes in the task checkpoint. Do not alter patch/resume-token semantics or add compression.
