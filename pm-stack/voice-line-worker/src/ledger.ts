// Turso (libsql) access for the voice line (#9800) over the Turso HTTP
// pipeline API — no SDK, just fetch, so the Worker stays small.
//
// Two tables (created idempotently by `ensureSchema`):
//   sessions          — the session ledger the issue requires: who asked
//                       (session id), what intent, what was answered,
//                       audio seconds in (submitted, post-trim) and out.
//   status_snapshots  — read-only status inputs for intents; the fleet
//                       pushes queue-state snapshots via /ingest-status
//                       and the queue_state intent answers from the
//                       newest row. kind is the snapshot family
//                       ("queue_state" for step 1).

import type { QueueSnapshot } from "./intent";

export interface SessionRow {
  id: string;
  startedAt: string;
  endedAt: string;
  transcript: string;
  intent: string;
  answer: string;
  audioSecondsIn: number;
  audioSecondsOut: number;
  sttModel: string;
  ttsModel: string;
}

type TursoValue =
  | { type: "null" }
  | { type: "text"; value: string }
  | { type: "float"; value: number }
  | { type: "integer"; value: string };

const text = (value: string): TursoValue => ({ type: "text", value });
const float = (value: number): TursoValue => ({ type: "float", value });

interface PipelineResult {
  results: Array<{
    type: string;
    response?: {
      type: string;
      // Turso HTTP pipeline v2: each row is a bare array of values,
      // NOT an object with a `values` field (verified live 2026-11-16,
      // task 16482 — reading `.values` off the row array returns
      // Array.prototype.values and destructuring it throws
      // "function is not iterable").
      result?: { cols: unknown[]; rows: TursoValue[][] };
    };
  }>;
}

/** Parse the newest queue_state snapshot out of a pipeline SELECT result. */
export function parseQueueSnapshotResult(out: PipelineResult): QueueSnapshot | null {
  const rows = out.results[0]?.response?.result?.rows ?? [];
  if (rows.length === 0) return null;
  const [payload, at] = rows[0];
  if (!payload || !at || payload.type !== "text" || at.type !== "text") return null;
  const parsed = JSON.parse(payload.value) as Omit<QueueSnapshot, "at">;
  return { ...parsed, at: at.value };
}

export class TursoClient {
  constructor(
    private url: string,
    private token: string,
  ) {}

  private async pipeline(statements: Array<{ sql: string; args: TursoValue[] }>): Promise<PipelineResult> {
    const res = await fetch(`${this.url.replace(/\/$/, "")}/v2/pipeline`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${this.token}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        requests: [...statements.map((stmt) => ({ type: "execute", stmt })), { type: "close" }],
      }),
    });
    if (!res.ok) throw new Error(`turso pipeline failed: HTTP ${res.status}`);
    return (await res.json()) as PipelineResult;
  }

  async execute(sql: string, args: TursoValue[] = []): Promise<void> {
    await this.pipeline([{ sql, args }]);
  }

  async ensureSchema(): Promise<void> {
    await this.pipeline([
      {
        sql: `CREATE TABLE IF NOT EXISTS sessions (
          id TEXT PRIMARY KEY, started_at TEXT NOT NULL, ended_at TEXT NOT NULL,
          transcript TEXT NOT NULL, intent TEXT NOT NULL, answer TEXT NOT NULL,
          audio_seconds_in REAL NOT NULL, audio_seconds_out REAL NOT NULL,
          stt_model TEXT NOT NULL, tts_model TEXT NOT NULL)`,
        args: [],
      },
      {
        sql: `CREATE TABLE IF NOT EXISTS status_snapshots (
          kind TEXT NOT NULL, payload TEXT NOT NULL, at TEXT NOT NULL)`,
        args: [],
      },
    ]);
  }

  async insertSession(row: SessionRow): Promise<void> {
    await this.execute(
      `INSERT INTO sessions (id, started_at, ended_at, transcript, intent, answer,
         audio_seconds_in, audio_seconds_out, stt_model, tts_model)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      [
        text(row.id),
        text(row.startedAt),
        text(row.endedAt),
        text(row.transcript),
        text(row.intent),
        text(row.answer),
        float(row.audioSecondsIn),
        float(row.audioSecondsOut),
        text(row.sttModel),
        text(row.ttsModel),
      ],
    );
  }

  async putSnapshot(kind: string, payload: unknown, at: string): Promise<void> {
    await this.execute(`INSERT INTO status_snapshots (kind, payload, at) VALUES (?, ?, ?)`, [
      text(kind),
      text(JSON.stringify(payload)),
      text(at),
    ]);
  }

  async latestQueueSnapshot(): Promise<QueueSnapshot | null> {
    const out = await this.pipeline([
      {
        sql: `SELECT payload, at FROM status_snapshots WHERE kind = 'queue_state'
              ORDER BY at DESC LIMIT 1`,
        args: [],
      },
    ]);
    return parseQueueSnapshotResult(out);
  }
}
