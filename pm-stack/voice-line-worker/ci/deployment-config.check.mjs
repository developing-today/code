import assert from "node:assert/strict";
import test from "node:test";
import config from "../cloudflare.config.ts";

test("cf config preserves the deployed Worker bindings and SQLite Durable Object", () => {
  assert.equal(config.worker.name, "voice-line");
  assert.equal(config.worker.compatibilityDate, "2025-09-15");
  assert.equal(config.worker.workersDev, true);
  assert.equal(config.worker.env.TURSO_URL.value, "https://voice-line-ledger-dezren39.aws-us-east-1.turso.io");
  assert.equal(config.worker.env.TURSO_TOKEN.type, "secret");
  assert.equal(config.worker.env.STATUS_INGEST_TOKEN.type, "secret");
  assert.deepEqual(config.worker.env.VOICE_SESSIONS, {
    type: "durable-object",
    worker: "voice-line",
    exportName: "VoiceSession",
  });
  assert.deepEqual(config.worker.env.AI, { type: "ai" });
  assert.deepEqual(config.worker.exports.VoiceSession, {
    type: "durable-object",
    storage: "sqlite",
  });
});
