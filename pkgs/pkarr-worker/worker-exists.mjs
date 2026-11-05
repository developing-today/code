import { Buffer } from "node:buffer";

/**
 * Check whether a Worker exists using a probe that throws on command failure.
 * Only cf's typed HTTP 404 response means the Worker is absent; all other
 * failures must stop deployment rather than select the bootstrap path.
 *
 * @param {string} workerName
 * @param {() => unknown} probe
 * @returns {boolean}
 */
export function workerExists(workerName, probe) {
  try {
    probe();
    return true;
  } catch (error) {
    const diagnostic = errorDiagnostic(error);
    if (isWorkerNotFound(diagnostic)) return false;

    throw new Error(
      `failed to probe Cloudflare Worker "${workerName}": ${diagnostic || "unknown error"}`,
      { cause: error },
    );
  }
}

function errorDiagnostic(error) {
  const parts = [
    error?.stderr,
    error?.stdout,
    error instanceof Error ? error.message : String(error),
  ].map(toText).filter(Boolean);

  return [...new Set(parts)].join("\n").trim();
}

function toText(value) {
  if (Buffer.isBuffer(value)) return value.toString("utf8");
  return typeof value === "string" ? value : "";
}

function isWorkerNotFound(diagnostic) {
  const typedNotFound = /\bNotFoundError\b/i.test(diagnostic);
  const http404 = /\b(?:status code|statusCode|http(?:\/[\d.]+)?)\s*[:=]?\s*404\b/i.test(diagnostic);
  return typedNotFound && http404;
}
