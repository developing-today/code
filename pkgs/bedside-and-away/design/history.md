# Safe history compensation

POST `/api/stores/:id/history` accepts `{expectedVersion,commandId,action:'undo'|'redo',targetEventId}`. The caller cannot provide before/after snapshots. The server retrieves the original event within this store and the last compensation for that original. Generic edits (`change`) and dedicated `variants` operations are eligible; steering, policy, and compensation events are not. Redo requires an earlier uncompensated undo. The same command ID retries idempotently.

Compensation traverses changed object fields and ID-keyed collections. Unrelated later field/entity edits survive. Every changed field must still equal the expected original endpoint, otherwise the operation returns 409 without mutation. Deleting a group created by the target event requires that group be unchanged; graph validation prevents leaving newer descendants orphaned. IDs and original record authors are restored from authoritative event data. Reorders require the same relative source order; unrelated inserted records are retained. Non-ID arrays are atomic fields.

The existing store CAS and atomic event+snapshot transaction persist a new `history.undo` or `history.redo` event, target referencing the original ID. Policy and steering state cannot be changed by compensation. Scoped stores remain read-only for history. No original event is deleted.

UI must submit this route and refetch. Build undo/redo buttons from eligible original change/variants events and subsequent history.undo/history.redo targeting them. Do not call generic /events with client-supplied undo snapshots; that route now rejects reserved operations. Reapply/merge remains a separately reviewed ordinary edit, not a historical undo.

Tests: keyed-object inverse, unrelated edits, touched-field conflict, order and ID preservation, group create compensation, lifecycle rejection, protected-state rejection, API actor/store bounds and 1,000 inverse/redo noninterference cases.
