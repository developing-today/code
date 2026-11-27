// Authenticated operator surface for the jurisdiction migration coordinator (#3404).
//
// This route is disabled unless a dedicated MIGRATION_OPERATOR_TOKEN is
// provisioned. It is an internal operational API, not an end-user route.

import { abortMigration, driveMigration, type MigrationPorts } from "./migration-coordinator.ts";
import { isMigrationId } from "./migration-fence.ts";
import { FailClosed, parseBindingId, parseJurisdiction } from "./routing.ts";

const MAX_BODY_BYTES = 4096;
const LEASE_TTL_MS = 60_000;
const PATH_PREFIX = "/v1/internal/migrations/";

export interface MigrationOperatorEnv {
  MIGRATION_OPERATOR_TOKEN?: string;
}

/** Match the bearer-token grammar at the configuration gate too, so a
 * whitespace-only or oversized secret remains cleanly disabled. */
export function migrationOperatorEnabled(
  env: MigrationOperatorEnv,
): env is MigrationOperatorEnv & { MIGRATION_OPERATOR_TOKEN: string } {
  return typeof env.MIGRATION_OPERATOR_TOKEN === "string" && /^\S{1,4096}$/.test(env.MIGRATION_OPERATOR_TOKEN);
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

async function sameToken(candidate: string, configured: string): Promise<boolean> {
  const encoder = new TextEncoder();
  const [left, right] = await Promise.all([
    crypto.subtle.digest("SHA-256", encoder.encode(candidate)),
    crypto.subtle.digest("SHA-256", encoder.encode(configured)),
  ]);
  const a = new Uint8Array(left);
  const b = new Uint8Array(right);
  let difference = 0;
  for (let i = 0; i < a.length; i++) difference |= a[i]! ^ b[i]!;
  return difference === 0;
}

async function bodyObject(req: Request): Promise<Record<string, unknown>> {
  const declared = req.headers.get("content-length");
  if (declared !== null && (!/^\d+$/.test(declared) || Number(declared) > MAX_BODY_BYTES)) {
    throw new FailClosed(413, "body_too_large", "migration operator request body exceeds 4096 bytes");
  }
  const reader = req.body?.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  if (reader) {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > MAX_BODY_BYTES) {
        await reader.cancel();
        throw new FailClosed(413, "body_too_large", "migration operator request body exceeds 4096 bytes");
      }
      chunks.push(value);
    }
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  let value: unknown;
  try {
    value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
  } catch {
    throw new FailClosed(400, "invalid_body", "migration operator request body must be JSON");
  }
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new FailClosed(400, "invalid_body", "migration operator request body must be a JSON object");
  }
  return value as Record<string, unknown>;
}

function exactKeys(value: Record<string, unknown>, keys: string[]): void {
  if (Object.keys(value).some((key) => !keys.includes(key)) || keys.some((key) => !(key in value))) {
    throw new FailClosed(400, "invalid_body", `request body must contain exactly: ${keys.join(", ")}`);
  }
}

function responseRecord(record: { migrationId: string | null; phase: string; jurisdiction: string; routingEpoch: number }) {
  return {
    migration_id: record.migrationId,
    phase: record.phase,
    jurisdiction: record.jurisdiction,
    routing_epoch: record.routingEpoch,
  };
}

/** Handle `/v1/internal/migrations/{start,resume,abort}`. The request is
 * authorized only by a dedicated secret; all other paths below the prefix
 * fail closed instead of falling through to a user Durable Object. */
export async function handleMigrationOperatorRequest(
  req: Request,
  env: MigrationOperatorEnv,
  ports: MigrationPorts,
): Promise<Response> {
  try {
    const url = new URL(req.url);
    if (!url.pathname.startsWith(PATH_PREFIX)) return json({ error: "not_found" }, 404);
    if (!migrationOperatorEnabled(env)) {
      throw new FailClosed(503, "migration_operator_disabled", "migration operator route is disabled");
    }
    const authorization = req.headers.get("authorization") ?? "";
    const match = /^Bearer (\S{1,4096})$/.exec(authorization);
    if (!match || !(await sameToken(match[1]!, env.MIGRATION_OPERATOR_TOKEN))) {
      throw new FailClosed(401, "operator_auth_required", "a valid migration operator bearer token is required");
    }
    if (req.method !== "POST") throw new FailClosed(405, "method_not_allowed", "migration operator routes require POST");

    const action = url.pathname.slice(PATH_PREFIX.length);
    if (action !== "start" && action !== "resume" && action !== "abort") {
      throw new FailClosed(404, "not_found", "no such migration operator route");
    }
    const body = await bodyObject(req);
    exactKeys(body, action === "abort" ? ["binding_id", "migration_id"] : ["binding_id", "migration_id", "target_jurisdiction"]);
    if (typeof body.binding_id !== "string") {
      throw new FailClosed(400, "invalid_binding_id", "binding_id must be a minted 32-character hexadecimal ID");
    }
    let bindingId: string;
    try {
      bindingId = parseBindingId(body.binding_id);
    } catch {
      throw new FailClosed(400, "invalid_binding_id", "binding_id must be a minted 32-character hexadecimal ID");
    }
    if (!isMigrationId(body.migration_id)) {
      throw new FailClosed(400, "invalid_migration_id", "migration_id must be 1 to 128 of A-Z a-z 0-9 . _ : -");
    }
    const migrationId = body.migration_id;
    const record = await ports.directory.findByBindingId(bindingId);
    if (record == null) throw new FailClosed(404, "directory_account_missing", "no hosted account record for this binding");

    const run = {
      subjectKey: record.subjectKey,
      migrationId,
      owner: `operator:${crypto.randomUUID()}`,
      leaseTtlMs: LEASE_TTL_MS,
    };
    if (action === "abort") {
      const result = await abortMigration(ports, run);
      return json({ result: "aborted", account: responseRecord(result) });
    }

    const targetJurisdiction = parseJurisdiction(
      typeof body.target_jurisdiction === "string" ? body.target_jurisdiction : null,
    );
    if (record.phase !== "stable" && (record.migrationId !== migrationId || record.jurisdictionPending !== targetJurisdiction)) {
      throw new FailClosed(409, "migration_in_progress", "binding has a different migration or target in progress");
    }
    if (action === "resume" && record.phase === "stable" && record.jurisdiction !== targetJurisdiction) {
      throw new FailClosed(409, "migration_not_in_progress", "resume requires the requested migration to be in progress or already complete");
    }
    const result = await driveMigration(ports, { ...run, targetJurisdiction });
    return json({ result: "complete", account: responseRecord(result) });
  } catch (error) {
    if (error instanceof FailClosed) return json({ error: error.code, message: error.message }, error.status);
    throw error;
  }
}
