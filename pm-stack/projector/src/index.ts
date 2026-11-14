// Entry point: migrate + serve the projector HTTP API.
import { openDb, migrate } from "./store";
import { serve, type ServerEnv } from "./server";
import { parsePort } from "./config";

const env = process.env as unknown as ServerEnv;
const port = parsePort(env.PORT);

const db = openDb(env);
await migrate(db);

// Dedicated clients so /health and the reconcile trigger never queue
// behind each other's traffic on one serialized libSQL client (#9850).
const healthDb = openDb(env);
const reconcileDb = openDb(env);

const server = serve(db, env, port, { healthDb, reconcileDb });
console.log(`[projector] listening on http://${server.hostname}:${server.port}`);
console.log(`[projector] database: ${process.env.DATABASE_URL ?? "file:data/projector.db"}`);
console.log(`[projector] repos: ${(env.PM_REPOS ?? "").split(",").filter(Boolean).join(", ") || "(accept all)"}`);
