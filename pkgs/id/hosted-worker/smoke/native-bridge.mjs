// Live integration smoke: run the native id-sync WebSocket client
// against the real local UserDO hosted worker.
import { spawn } from "node:child_process";
import { createPrivateKey, createPublicKey } from "node:crypto";
import { once } from "node:events";

const PKG = new URL("..", import.meta.url).pathname;
const ID_ROOT = new URL("../../", import.meta.url).pathname;
const PORT = 8799;
const BINDING = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const seed = Buffer.alloc(32, 9);
const privateKey = createPrivateKey({
  key: Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), seed]),
  format: "der",
  type: "pkcs8",
});
const iroh = createPublicKey(privateKey).export({ format: "der", type: "spki" }).subarray(-32).toString("hex");
const worker = spawn("wrangler", ["dev", "-c", "wrangler.test.toml", "--port", String(PORT), "--local"], {
  cwd: PKG,
  stdio: ["ignore", "ignore", "inherit"],
  detached: true,
});

async function stop() {
  try { process.kill(-worker.pid, "SIGTERM"); } catch {}
  await Promise.race([once(worker, "exit"), new Promise((resolve) => setTimeout(resolve, 1500))]);
}

try {
  let ready = false;
  for (let i = 0; i < 45 && !ready; i++) {
    try {
      const response = await fetch(`http://127.0.0.1:${PORT}/v1/state?name=${BINDING}&binding=${BINDING}&jurisdiction=us`);
      ready = response.status === 200;
    } catch {}
    if (!ready) await new Promise((resolve) => setTimeout(resolve, 1000));
  }
  if (!ready) throw new Error("test UserDO worker did not become ready");

  const url = new URL(`ws://127.0.0.1:${PORT}/v1/bridge/ws`);
  url.searchParams.set("name", BINDING);
  url.searchParams.set("binding", BINDING);
  url.searchParams.set("jurisdiction", "us");
  url.searchParams.set("iroh", iroh);
  const result = await new Promise((resolve, reject) => {
    const cargo = spawn("cargo", [
      "test", "-p", "id-sync", "bridge::tests::live_hosted_worker_accepts_native_push_and_resume",
      "--", "--ignored", "--exact",
    ], {
      cwd: ID_ROOT,
      stdio: "inherit",
      env: { ...process.env, ID_HOSTED_BRIDGE_WS_URL: url.toString() },
    });
    cargo.once("error", reject);
    cargo.once("exit", (code) => code === 0 ? resolve() : reject(new Error(`cargo test exited ${code}`)));
  });
  void result;
  console.log("LIVE NATIVE BRIDGE OK");
} finally {
  await stop();
}
