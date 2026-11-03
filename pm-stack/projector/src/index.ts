// Entry point: migrate + serve the projector HTTP API.
import { openDb, migrate } from "./store";
import { serve, type ServerEnv } from "./server";

const env = process.env as unknown as ServerEnv;
const port = Number(env.PORT ?? 7801);

const db = openDb(env);
await migrate(db);

const server = serve(db, env, port);
console.log(`[projector] listening on http://${server.hostname}:${server.port}`);
console.log(`[projector] database: ${process.env.DATABASE_URL ?? "file:data/projector.db"}`);
console.log(`[projector] repos: ${(env.PM_REPOS ?? "").split(",").filter(Boolean).join(", ") || "(accept all)"}`);
