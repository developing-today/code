// Turso management operations, started from the app (#14257).
//
// Creating a database and minting a token need the Turso organisation's
// token. That token is never held by the Worker or this console: it
// lives in the repository's secret store under the name
// TURSO_PLATFORM_API_TOKEN and is read by name only in the one workflow
// that performs these operations (pm-turso-admin.yml), in the runner
// only. This module is the console side of that split: it validates an
// operation, builds the workflow's dispatch inputs (names and choices,
// never a token value), and reads a run's outcome so a failure is
// surfaced as a failure, not a silent no-op.

import { OpError } from "./errors.ts";

export const TURSO_ADMIN_WORKFLOW = "pm-turso-admin.yml";
/** The repository secret that holds the Turso organisation token.
 * Referenced by name only, and only in TURSO_ADMIN_WORKFLOW. */
export const TURSO_ORG_TOKEN_SECRET = "TURSO_PLATFORM_API_TOKEN";
/** The one Turso group every fleet database is born in
 * (doc/fleet/turso-databases.md; pm-stack/book/reference/data-stores.md). */
export const TURSO_GROUP = "great-star-lord";

export const TURSO_OPERATIONS = ["create-db", "mint-token"] as const;
export type TursoOperation = (typeof TURSO_OPERATIONS)[number];

function isTursoOperation(value: unknown): value is TursoOperation {
  return TURSO_OPERATIONS.some((operation) => operation === value);
}

const DATABASE = /^[a-z0-9][a-z0-9-]{0,62}$/;
const SECRET_NAME = /^[A-Za-z_][A-Za-z0-9_]{0,254}$/;
const EXPIRY = /^([0-9]+[dwmy]|never)$/;
const FORBIDDEN_SECRET_NAMES = new Set([TURSO_ORG_TOKEN_SECRET, "REPO_AUTOMATION_TOKEN"]);

function validSecretName(value: string): boolean {
  return (
    SECRET_NAME.test(value) &&
    !value.toUpperCase().startsWith("GITHUB_") &&
    !FORBIDDEN_SECRET_NAMES.has(value.toUpperCase())
  );
}

export interface TursoAdminRequest {
  operation: TursoOperation;
  database: string;
  /** create-db only: the group the database is born in. */
  group: string | null;
  /** mint-token only: how long the minted token lives. */
  expiry: string | null;
  /** mint-token only: the repository secret the minted token is
   * written straight into. A name, never a value. */
  secretName: string | null;
}

/** Validate console arguments into one admin operation. Anything the
 * workflow must not be asked to do is refused here, before dispatch. */
export function tursoAdminRequest(args: Record<string, unknown>): TursoAdminRequest {
  const operation = args.operation;
  if (!isTursoOperation(operation)) {
    throw new OpError(400, "operation: create-db or mint-token");
  }
  const database = args.database;
  if (typeof database !== "string" || !DATABASE.test(database)) {
    throw new OpError(400, "database: a Turso database name (lowercase letters, digits, hyphens)");
  }
  if (operation === "create-db") {
    const group = args.group ?? TURSO_GROUP;
    if (group !== TURSO_GROUP) throw new OpError(400, `group: the fleet's one group, ${TURSO_GROUP}`);
    return { operation, database, group: TURSO_GROUP, expiry: null, secretName: null };
  }
  const expiry = args.expiry ?? "4w";
  if (typeof expiry !== "string" || !EXPIRY.test(expiry)) {
    throw new OpError(400, "expiry: a span like 4w, 30d or never");
  }
  const secretName = args.secret_name;
  if (typeof secretName !== "string" || !validSecretName(secretName)) {
    throw new OpError(400, "secret_name: the name of the repository secret to hold the minted token");
  }
  return { operation, database, group: null, expiry, secretName };
}

/** The workflow's dispatch inputs for a validated request: text only,
 * as GitHub takes them, and never any token value. */
export function tursoAdminInputs(req: TursoAdminRequest): Record<string, string> {
  const inputs: Record<string, string> = { operation: req.operation, database: req.database };
  if (req.group !== null) inputs.group = req.group;
  if (req.expiry !== null) inputs.expiry = req.expiry;
  if (req.secretName !== null) inputs.secret_name = req.secretName;
  return inputs;
}

export interface TursoAdminRun {
  state: string;
  /** True only when the run finished and did not succeed: the caller
   * must show it as a failure. */
  failed: boolean;
  at: string;
  url: string;
}

/** One workflow run, as GitHub's record gives it, read for the Stores
 * section: while it runs, where it is; once over, how it ended — a
 * failure stays a failure. */
export function tursoAdminRun(run: {
  status: string;
  conclusion: string | null;
  created_at: string;
  html_url: string;
}): TursoAdminRun {
  const state = run.status === "completed" ? (run.conclusion ?? "unknown") : run.status;
  return { state, failed: run.status === "completed" && state !== "success", at: run.created_at, url: run.html_url };
}
