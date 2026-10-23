# Per-session world viewport and paste policy

## Intent

Give each browser or SSH participant an appropriate bounded presentation without allowing one participant's resize to mutate shared application state. Define a safe explicit policy for clipboard input.

## Contract

- `resize {width, height}` is connection-local presentation context, not a `WorldEvent`. The host accepts widths up to 500 columns and heights up to 200 rows.
- A structured screen is cropped to the connection's viewport. Cropping through a wide glyph substitutes a blank cell rather than emitting a malformed grid. Legacy text remains unchanged.
- Browser dimensions come from a `ResizeObserver`; SSH dimensions come from the initial PTY request and subsequent SSH window-change requests.
- `paste {text}` normalizes CRLF and CR to LF, accepts at most 4096 UTF-8 bytes, allows line-feed and tab, and rejects other control characters. Accepted paste is one ordinary durable input event, preserving deterministic replay.
- The existing terminal key path remains unchanged. Browser paste is intercepted only while the world screen is focused; SSH terminal paste arrives as PTY bytes.

## Deliberate limit

The guest sees only the existing state-driven view function and does not receive viewport dimensions. This phase clips a fixed screen per connection; it does not reflow Roc applications. App-level responsive layout would require a separately versioned guest ABI/context mechanism. Resize and focus are never broadcast or journaled.

## Acceptance

- Resize does not advance the world sequence or appear in replayed events.
- Two participants can use different viewports without affecting each other's presentation.
- Screen clipping remains a valid bounded cell grid, including wide-glyph boundaries.
- Paste is bounded, normalized, rejects unsupported control characters, and enters through the established `Input` event.
- Browser and SSH resize updates are scoped to their individual session.
