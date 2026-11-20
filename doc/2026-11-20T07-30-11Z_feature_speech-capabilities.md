# `speech.*` — conversational audio capabilities

Status: implementation slice for issue #2344. This records the requested
provider-neutral contract before code changes. The capability names are
`speech.transcribe`, `speech.capture`, and `speech.synthesize`; their runtime
providers and device-specific execution remain adapter choices.

## Capabilities and default path

| Capability | Contract |
| --- | --- |
| `speech.transcribe` | Transcribe on the client/device by default. Audio stays on that device; the resulting text enters the existing `chat.say`-shaped or form-value path. |
| `speech.capture` | Optional per-app server-side audio capture for devices without local STT or apps that choose a server provider. It requires consent and uses the existing `media.stream` frame-size and consent rules. |
| `speech.synthesize` | Turn text into speech through a runtime-selected provider. The taxonomy does not bind this capability to a vendor or a particular device side. |

The world model consumes text, not a new audio-message type. Client-side
transcription is therefore the default; enabling server capture is an explicit
per-app choice, not a global setting.

## Refusals

| Situation | Code | Reason |
| --- | --- | --- |
| Required capture consent has not been given | `cap_denied` | `consent_required` |
| The device has no microphone affordance | `unknown_cap` | `device_lacks_affordance` |
| The selected ASR provider is unavailable | `failed` | `asr_unavailable` |

These add reason vocabulary while preserving the stable capability error
codes. Consent is not itself a durable grant: the grant ledger and the
runtime consent decision remain separate.

## Audio and retention

Captured frames must not exceed the `media.stream` frame ceiling, and capture
must obey its consent rules. Audio bytes are not written to the journal by
default. Retention is possible only when the program explicitly requests an
artifact, following the [`media.*` artifact contract](https://github.com/developing-today/code/issues/2342); otherwise
the stream is discarded after use.

## Scope boundaries

Speech providers are runtime-selected and provider-neutral. This slice does
not define a new message type, speaker diarisation, wake-word detection,
continuous conversation loops, or TTS voice selection.

The taxonomy investigation's speech design is in
[`doc/2026-10-29T14-05-00Z_investigation_capability_taxonomy/2026-10-29T14-05-00Z_investigation_capability_taxonomy.md`](2026-10-29T14-05-00Z_investigation_capability_taxonomy/2026-10-29T14-05-00Z_investigation_capability_taxonomy.md#35-speech-human-conversation-two-audio-paths).
