// Control-frame parsing for the VoiceSession WebSocket (#10921).
//
// A text frame must be a JSON object. JSON.parse accepts null, numbers,
// strings and arrays too, and reading `.type` off null throws, so every
// non-object value is rejected here with an error the client can see,
// instead of escaping the handler as an uncaught TypeError.

export type ControlFrame = { ok: true; type: unknown } | { ok: false; error: string };

export function parseControlFrame(message: string): ControlFrame {
  let value: unknown;
  try {
    value = JSON.parse(message);
  } catch {
    return { ok: false, error: "control frame must be JSON" };
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return { ok: false, error: "control frame must be a JSON object" };
  }
  return { ok: true, type: (value as { type?: unknown }).type };
}
