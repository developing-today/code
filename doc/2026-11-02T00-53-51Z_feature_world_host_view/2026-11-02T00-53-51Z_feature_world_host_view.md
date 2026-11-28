# Authenticated, read-only world host view

## Request

Issue [#3378](https://github.com/developing-today/code/issues/3378) asks for an explicitly authenticated host to inspect the current authoritative world view and records without joining as a guest or creating, minting, or consuming a guest capability. The existing guest-capability flow must remain unchanged. The existing Wasm/Roc installation controls should be distinct and discoverable from the empty-program state.

## Initial design

- Use the existing world-admin secret, sent only in the `x-world-admin-token` HTTP header, to authorize a web-only read endpoint. Do not put credentials in a URL or add a new shell-protocol frame: `shell-contract.v1.json` explicitly defers new frame kinds.
- Add an actor-serialized admin view/snapshot operation. The program's existing `view(Participant)` interface needs a view context, so use a reserved host viewer (`id = 0`, display name `Host`) only for rendering; do not register it, include it in the participant list, emit presence, or allow it to send events.
- Return a bounded page of records and a continuation key. The browser fetches pages using the same admin header, renders the host view as read-only, and can refresh it explicitly.
- Keep the guest socket/join and its capability authorization intact. Host mode closes any guest session and disables chat/input/paste/keyboard sends.
- Move installation controls into a separate “Install or update program” host section. Use `active_module_hash` to show an actionable empty-program state.

## Trade-offs and checks

The web-only endpoint avoids changing the shared QUIC/WebSocket shell contract. The admin secret is already used by the web host tools and is the authority boundary for this operation. The reserved viewer is never a guest or participant; this preserves the roster/presence invariant while reusing the current program presentation interface. Verify authorization denial, no host participant/presence, unchanged guest join, bounded record pagination, and disabled host input.

## Change log

- 2026-11-02: Initial design before implementation for queue task 5190.
