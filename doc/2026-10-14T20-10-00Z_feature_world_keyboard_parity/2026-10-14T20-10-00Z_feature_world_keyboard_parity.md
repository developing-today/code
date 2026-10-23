# Browser and SSH keyboard parity for world programs

## Intent

World apps already receive a journaled `Input(Vec<u8>)` event. SSH PTY bytes enter that existing path unchanged, and the shared Roc `Key.parse` helper understands printable UTF-8, control keys, and common terminal escape sequences. The browser currently offers only a text input field. This phase makes the structured screen focusable and sends its keydown events using the same terminal byte sequences, with no new host authority or event type.

## Interface

- Focus the world screen and press supported keys. Browser keys are encoded to the same bytes a terminal normally sends: UTF-8 text, CR/tab/backspace/escape, and standard CSI/SS3 navigation sequences.
- Browser shortcuts using Meta are left to the browser/OS. Unsupported keys are ignored instead of sending misleading input.
- The byte payload is sent through the existing `input` frame and remains a durable world event, exactly like SSH input. Apps can use the existing Roc `Key.parse` module.
- Chat and the explicit text-input form remain available, and SSH wire input is unchanged.

## Decisions and scope boundary

Key events are deliberately durable because they affect game/application state and must replay deterministically. Resize and focus are presentation context, not shared game events; journaling one participant's terminal size would let it resize every participant's shared world state. Per-session viewport-aware rendering therefore needs a separate non-journaled context path and is not included here. Paste is left to the explicit text-input field until its size/normalization policy is defined.

## Acceptance

- The browser screen is keyboard focusable and accessible.
- Common keys encode to exact terminal byte strings that `Key.parse` recognizes.
- Meta shortcuts are not intercepted; ordinary UI inputs do not leak key events into the world.
- Key bytes use the current bounded input frame and are replayed as existing `Input` events.
- Existing text input and SSH behavior remain unchanged.
