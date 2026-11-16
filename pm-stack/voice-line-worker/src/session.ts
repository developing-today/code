// VoiceSession Durable Object (#9800): one push-to-talk session.
//
// Protocol (see README.md): the client opens a WebSocket, streams
// PCM16LE 16 kHz mono audio as binary frames while the talk button is
// held, then sends {"type":"stop"}. The DO trims silence (src/vad.ts),
// transcribes the trimmed audio with Workers AI Whisper, matches the
// one read-only intent (src/intent.ts), answers in text AND speech
// (Workers AI MeloTTS — the line talks back, per Drew's 2026-11-14
// amendment), and writes the session ledger row to Turso
// (src/ledger.ts) before replying {"type":"done"}.
//
// Audio is buffered in memory for the life of the socket: a step-1
// simplification, sized by push-to-talk utterances (seconds, not
// minutes). Hibernation-safe storage chunking is a step-2 concern and
// is deliberately not faked here.

import { trimSilence, pcmSeconds } from "./vad";
import { matchIntent, buildAnswer } from "./intent";
import { TursoClient } from "./ledger";

export interface Env {
  AI: Ai;
  VOICE_SESSIONS: DurableObjectNamespace;
  TURSO_URL: string;
  TURSO_TOKEN: string;
}

export const STT_MODEL = "@cf/openai/whisper-large-v3-turbo";
export const TTS_MODEL = "@cf/myshell-ai/melotts";
export const SAMPLE_RATE = 16000;

function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

function base64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out;
}

export function wavEncodePcm16(pcm: Uint8Array, sampleRate: number): Uint8Array {
  // Wrap raw PCM16LE mono in a 44-byte RIFF/WAVE header. Whisper
  // rejects headerless PCM with AiError "Invalid input" (code 8001) —
  // found live during the task 16482 deploy proof, 2026-11-16: the
  // binding call threw, the utterance died silently, no ledger row.
  const out = new Uint8Array(44 + pcm.byteLength);
  const view = new DataView(out.buffer);
  const writeAscii = (off: number, text: string) => {
    for (let i = 0; i < text.length; i++) view.setUint8(off + i, text.charCodeAt(i));
  };
  writeAscii(0, "RIFF");
  view.setUint32(4, 36 + pcm.byteLength, true);
  writeAscii(8, "WAVE");
  writeAscii(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  writeAscii(36, "data");
  view.setUint32(40, pcm.byteLength, true);
  out.set(pcm, 44);
  return out;
}

export class VoiceSession {
  private chunks: Uint8Array[] = [];
  private startedAt = new Date().toISOString();

  constructor(
    private state: DurableObjectState,
    private env: Env,
  ) {}

  async fetch(request: Request): Promise<Response> {
    if (request.headers.get("Upgrade") !== "websocket") {
      return new Response("expected websocket upgrade", { status: 426 });
    }
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    this.state.acceptWebSocket(server);
    server.serializeAttachment({ sessionId: new URL(request.url).searchParams.get("session") ?? crypto.randomUUID() });
    return new Response(null, { status: 101, webSocket: client });
  }

  private send(ws: WebSocket, message: Record<string, unknown>): void {
    ws.send(JSON.stringify(message));
  }

  async webSocketMessage(ws: WebSocket, message: string | ArrayBuffer): Promise<void> {
    if (typeof message !== "string") {
      this.chunks.push(new Uint8Array(message));
      return;
    }
    let control: { type?: string };
    try {
      control = JSON.parse(message);
    } catch {
      this.send(ws, { type: "error", error: "control frame must be JSON" });
      return;
    }
    if (control.type === "start") {
      this.chunks = [];
      this.startedAt = new Date().toISOString();
      this.send(ws, { type: "ready" });
    } else if (control.type === "stop") {
      try {
        await this.processUtterance(ws);
      } catch (err) {
        this.send(ws, {
          type: "error",
          error: `processing failed: ${err instanceof Error ? err.message : String(err)}`,
        });
      }
    } else {
      this.send(ws, { type: "error", error: `unknown control type: ${control.type ?? "(none)"}` });
    }
  }

  private async processUtterance(ws: WebSocket): Promise<void> {
    const { sessionId } = ws.deserializeAttachment() as { sessionId: string };
    const rawLength = this.chunks.reduce((n, c) => n + c.byteLength, 0);
    const raw = new Uint8Array(rawLength);
    let offset = 0;
    for (const c of this.chunks) {
      raw.set(c, offset);
      offset += c.byteLength;
    }
    this.chunks = [];
    const secondsInRaw = pcmSeconds(raw.byteLength, SAMPLE_RATE);

    const trimmed = trimSilence(raw, SAMPLE_RATE);
    if (trimmed.allSilence) {
      this.send(ws, { type: "error", error: "no speech detected (all silence after VAD)" });
      return;
    }

    // Speech-to-text: ONLY the trimmed audio is submitted (never billed
    // for silence — #9800).
    const stt = (await this.env.AI.run(STT_MODEL, {
      audio: bytesToBase64(wavEncodePcm16(trimmed.audio, SAMPLE_RATE)),
    })) as { text?: string };
    const transcript = (stt.text ?? "").trim();
    this.send(ws, {
      type: "transcript",
      text: transcript,
      audioSecondsSubmitted: trimmed.secondsSubmitted,
      audioSecondsRaw: secondsInRaw,
    });

    const turso = new TursoClient(this.env.TURSO_URL, this.env.TURSO_TOKEN);
    const intent = matchIntent(transcript);
    const snapshot = intent === "queue_state" ? await turso.latestQueueSnapshot() : null;
    const answer = buildAnswer(intent, snapshot);
    this.send(ws, { type: "answer", intent, text: answer });

    // Speech out: MeloTTS speaks the answer in the same session.
    const tts = (await this.env.AI.run(TTS_MODEL, { prompt: answer })) as { audio?: string };
    let secondsOut = 0;
    if (tts.audio) {
      const audioBytes = base64ToBytes(tts.audio);
      secondsOut = Math.max(1, Math.round(answer.split(/\s+/).length / 2.5));
      ws.send(audioBytes.buffer as ArrayBuffer);
    }

    await turso.ensureSchema();
    await turso.insertSession({
      id: sessionId,
      startedAt: this.startedAt,
      endedAt: new Date().toISOString(),
      transcript,
      intent,
      answer,
      audioSecondsIn: trimmed.secondsSubmitted,
      audioSecondsOut: secondsOut,
      sttModel: STT_MODEL,
      ttsModel: TTS_MODEL,
    });
    this.send(ws, {
      type: "done",
      sessionId,
      intent,
      audioSecondsIn: trimmed.secondsSubmitted,
      audioSecondsOut: secondsOut,
    });
  }

  async webSocketClose(ws: WebSocket, code: number): Promise<void> {
    ws.close(code);
  }

  async webSocketError(ws: WebSocket): Promise<void> {
    ws.close(1011);
  }
}
