// lane-scratch: code-c /tmp/lane-code-c-20261116-0412 task 16482
// WS proof client for the deployed voice-line worker (#9800 step 1):
// streams a PCM16LE 16kHz file as one push-to-talk utterance and prints
// every server event; binary frames are reported by byte length only.
const url = process.argv[2];
const pcmPath = process.argv[3];
if (!url || !pcmPath) throw new Error("usage: bun ws-proof.ts <wss-url> <pcm-file>");
const pcm = await Bun.file(pcmPath).arrayBuffer();
const ws = new WebSocket(url);
const done = new Promise<void>((resolve, reject) => {
  const timer = setTimeout(() => reject(new Error("timeout waiting for done")), 90000);
  ws.addEventListener("message", (ev: MessageEvent) => {
    if (typeof ev.data === "string") {
      console.log("EVENT", ev.data);
      try {
        const j = JSON.parse(ev.data);
        if (j.type === "done" || j.type === "error") { clearTimeout(timer); resolve(); }
      } catch { /* ignore */ }
    } else if (ev.data instanceof Blob) {
      ev.data.arrayBuffer().then((b) => console.log("BINARY", b.byteLength, "bytes"));
    } else if (ev.data instanceof ArrayBuffer) {
      console.log("BINARY", ev.data.byteLength, "bytes");
    }
  });
  ws.addEventListener("error", (e) => { clearTimeout(timer); reject(new Error("ws error " + String(e))); });
});
await new Promise<void>((resolve, reject) => {
  ws.addEventListener("open", () => resolve(), { once: true });
  ws.addEventListener("error", () => reject(new Error("ws open failed")), { once: true });
});
ws.send(JSON.stringify({ type: "start" }));
// stream in ~100ms frames (3200 bytes = 100ms of 16kHz PCM16 mono)
for (let off = 0; off < pcm.byteLength; off += 3200) {
  ws.send(pcm.slice(off, off + 3200));
}
ws.send(JSON.stringify({ type: "stop" }));
await done; await new Promise(r => setTimeout(r, 1500));
ws.close();
console.log("PROOF_DONE");
