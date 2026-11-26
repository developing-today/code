// Bitwarden Secrets Manager collector core (#1111, hub design §2.2).
//
// The count-only read: `bws secret list` and `bws project list` answer
// JSON arrays; this module reads ONLY the array lengths. Secret and
// project names, keys, and values are never parsed into fields, never
// logged, and never enter the hub — the parsed elements are discarded
// the moment the length is taken. The dated counts are written into
// the tracker's Bitwarden section in exactly the phrasing
// saas-extract's bitwardenMetrics parses, so the read flows into the
// /quota card and the §4 snapshot records with no further wiring.
//
// The bws access token is the caller's environment concern (the CLI
// script passes its own env through to the child process); no token
// value is accepted, stored, or printed here.
import { parseIsoDate } from "./validate.ts";

export interface BitwardenCounts {
  secrets: number;
  projects: number;
}

export class BwsCountError extends Error {}

/** Count one `bws ... list` JSON document by array length only. The
 * elements are never inspected beyond the array check, so no name,
 * key, or value can leak into a return value, log, or error. */
export function countBwsListOutput(jsonText: string, kind: "secret" | "project"): number {
  let parsed: unknown;
  try {
    parsed = JSON.parse(jsonText);
  } catch {
    throw new BwsCountError(`bws ${kind} list output is not valid JSON; no count recorded`);
  }
  if (!Array.isArray(parsed)) {
    throw new BwsCountError(`bws ${kind} list output is not a JSON array; no count recorded`);
  }
  return parsed.length;
}

export function countsFromBwsOutputs(secretListJson: string, projectListJson: string): BitwardenCounts {
  return {
    secrets: countBwsListOutput(secretListJson, "secret"),
    projects: countBwsListOutput(projectListJson, "project"),
  };
}

/** Runs the two count-only list commands through an injectable runner
 * (the CLI script supplies the real `bws` spawn; tests supply a fake).
 * The runner returns the command's stdout; any runner failure is
 * wrapped without the command output, which is never echoed. */
export function collectBitwardenCounts(run: (args: string[]) => string): BitwardenCounts {
  let secretJson: string;
  let projectJson: string;
  try {
    secretJson = run(["secret", "list"]);
    projectJson = run(["project", "list"]);
  } catch {
    throw new BwsCountError("bws list command failed; no count recorded");
  }
  return countsFromBwsOutputs(secretJson, projectJson);
}

function requireIsoDate(date: string): string {
  if (!parseIsoDate(date)) {
    throw new BwsCountError(`read date ${JSON.stringify(date)} is not a real ISO calendar date`);
  }
  return date;
}

/** The Usage-bullet value bitwardenMetrics parses, e.g.
 * "212 secrets, 1 project (bws count-only read 2026-11-22)". */
export function formatBitwardenUsageValue(counts: BitwardenCounts, date: string): string {
  requireIsoDate(date);
  const secrets = `${counts.secrets} secret${counts.secrets === 1 ? "" : "s"}`;
  const projects = `${counts.projects} project${counts.projects === 1 ? "" : "s"}`;
  return `${secrets}, ${projects} (bws count-only read ${date})`;
}

/**
 * Record one dated count-only read in the tracker view: replace the
 * Bitwarden section's Usage/dates and Last checked bullets with the
 * new read. Fails closed (throws, view untouched) when the section or
 * either bullet is missing — a collector must never guess where a
 * read belongs. Everything outside those two bullet lines is returned
 * byte-identical.
 */
export function applyBitwardenRead(view: string, counts: BitwardenCounts, date: string): string {
  requireIsoDate(date);
  const lines = view.split("\n");
  const start = lines.findIndex((l) => /^### Bitwarden\s*$/.test(l));
  if (start < 0) throw new BwsCountError("tracker view has no '### Bitwarden' section; read not recorded");
  let end = lines.length;
  for (let i = start + 1; i < lines.length; i++) {
    if (/^#{2,3}\s/.test(lines[i]!)) {
      end = i;
      break;
    }
  }
  let usageIdx = -1;
  let checkedIdx = -1;
  for (let i = start + 1; i < end; i++) {
    if (lines[i]!.startsWith("- **Usage / dates:**")) usageIdx = i;
    if (lines[i]!.startsWith("- **Last checked:**")) checkedIdx = i;
  }
  if (usageIdx < 0 || checkedIdx < 0) {
    throw new BwsCountError("Bitwarden section lacks a Usage/dates or Last checked bullet; read not recorded");
  }
  const out = [...lines];
  out[usageIdx] = `- **Usage / dates:** ${formatBitwardenUsageValue(counts, date)}.`;
  out[checkedIdx] = `- **Last checked:** bws count-only read ${date}.`;
  return out.join("\n");
}
