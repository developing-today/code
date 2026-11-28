// Voice line Worker entry (#9800): serves the push-to-talk page,
// routes WebSocket sessions to the VoiceSession Durable Object, and
// accepts read-only status snapshots the intents answer from.
//
// Routes:
//   GET  /               push-to-talk client page
//   GET  /ws?session=<id>  WebSocket upgrade -> VoiceSession DO
//   GET  /health         { ok, latestQueueSnapshotAt }
//   GET  /version        { ok, service, version } — the deployed
//                        commit from the VERSION var (#15793)
//   POST /ingest-status  Bearer STATUS_INGEST_TOKEN; body is a
//                        queue-state snapshot {queued, claimed, parked,
//                        blocked} stored for the queue_state intent.
//                        This is a status INPUT, not a voice write
//                        action: the voice line itself stays read-only.

import { TursoClient } from "./ledger";
import { versionAnswer } from "./version";
import type { Env } from "./session";
import { parseIngestStatus } from "./ingest";

export { VoiceSession } from "./session";

const PAGE = `<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Voice line — step 1 (#9800)</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>body{font-family:system-ui;margin:2rem;max-width:40rem}button{font-size:1.25rem;padding:1rem 2rem}
#log{white-space:pre-wrap;background:#111;color:#eee;padding:1rem;border-radius:.5rem;min-height:8rem}</style>
</head>
<body>
<h1>Voice line — step 1</h1>
<p>Hold the button and ask about the fleet queue, e.g. &ldquo;how many tasks are in the queue?&rdquo;
Your speech is trimmed of silence, transcribed, answered, and spoken back.</p>
<button id="talk" disabled>Connecting…</button>
<div id="log"></div>
<script>
const log = (m) => { document.getElementById('log').textContent += m + '\\n'; };
const sessionId = crypto.randomUUID();
const ws = new WebSocket(location.origin.replace('http', 'ws') + '/ws?session=' + sessionId);
let audioCtx, source, processor, stream, chunks;
ws.binaryType = 'arraybuffer';
ws.onopen = () => { const b = document.getElementById('talk'); b.disabled = false; b.textContent = 'Hold to talk'; };
ws.onmessage = async (ev) => {
  if (typeof ev.data !== 'string') {
    const ctx = new AudioContext();
    const buf = await ctx.decodeAudioData(ev.data);
    const src = ctx.createBufferSource(); src.buffer = buf; src.connect(ctx.destination); src.start();
    log('🔊 speaking the answer');
    return;
  }
  const msg = JSON.parse(ev.data);
  if (msg.type === 'transcript') log('📝 ' + msg.text + ' (' + msg.audioSecondsSubmitted.toFixed(1) + 's submitted of ' + msg.audioSecondsRaw.toFixed(1) + 's recorded)');
  else if (msg.type === 'answer') log('💬 ' + msg.text);
  else if (msg.type === 'done') log('✅ session ' + msg.sessionId + ' written to the ledger');
  else if (msg.type === 'error') log('⚠️ ' + msg.error);
};
async function start() {
  stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  audioCtx = new AudioContext({ sampleRate: 16000 });
  source = audioCtx.createMediaStreamSource(stream);
  processor = audioCtx.createScriptProcessor(4096, 1, 1);
  chunks = [];
  processor.onaudioprocess = (e) => {
    const f = e.inputBuffer.getChannelData(0);
    const pcm = new Int16Array(f.length);
    for (let i = 0; i < f.length; i++) pcm[i] = Math.max(-32768, Math.min(32767, Math.round(f[i] * 32767)));
    ws.send(pcm.buffer);
  };
  ws.send(JSON.stringify({ type: 'start' }));
  source.connect(processor); processor.connect(audioCtx.destination);
}
async function stop() {
  processor.disconnect(); source.disconnect(); stream.getTracks().forEach(t => t.stop());
  await audioCtx.close();
  ws.send(JSON.stringify({ type: 'stop' }));
}
const btn = document.getElementById('talk');
btn.addEventListener('pointerdown', start);
btn.addEventListener('pointerup', stop);
</script>
</body>
</html>`;

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);

    if (url.pathname === "/") {
      return new Response(PAGE, { headers: { "Content-Type": "text/html; charset=utf-8" } });
    }

    if (url.pathname === "/version") {
      if (request.method !== "GET") return new Response("method not allowed", { status: 405 });
      return Response.json(versionAnswer(env));
    }

    if (url.pathname === "/ws") {
      const sessionId = url.searchParams.get("session") ?? crypto.randomUUID();
      const stub = env.VOICE_SESSIONS.get(env.VOICE_SESSIONS.idFromName(sessionId));
      return stub.fetch(request);
    }

    if (url.pathname === "/health") {
      const turso = new TursoClient(env.TURSO_URL, env.TURSO_TOKEN);
      let latest: string | null = null;
      try {
        latest = (await turso.latestQueueSnapshot())?.at ?? null;
      } catch {
        latest = null;
      }
      return Response.json({ ok: true, latestQueueSnapshotAt: latest });
    }

    if (url.pathname === "/ingest-status" && request.method === "POST") {
      const expected = (env as Env & { STATUS_INGEST_TOKEN?: string }).STATUS_INGEST_TOKEN;
      if (!expected || request.headers.get("Authorization") !== `Bearer ${expected}`) {
        return new Response("unauthorized", { status: 401 });
      }
      // #11014: refuse malformed bodies and bad counts with a 400
      // instead of storing values the queue_state intent would speak.
      const parsed = parseIngestStatus(await request.text());
      if (!parsed.ok) {
        return Response.json(parsed.field ? { error: parsed.error, field: parsed.field } : { error: parsed.error }, {
          status: 400,
        });
      }
      const snapshot = parsed.snapshot;
      const turso = new TursoClient(env.TURSO_URL, env.TURSO_TOKEN);
      await turso.ensureSchema();
      await turso.putSnapshot("queue_state", snapshot, new Date().toISOString());
      return Response.json({ ok: true, stored: snapshot });
    }

    return new Response("not found", { status: 404 });
  },
};
