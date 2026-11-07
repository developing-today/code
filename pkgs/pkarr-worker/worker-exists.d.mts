/**
 * Check whether a Worker exists using a probe that throws on command failure.
 * Only cf's typed HTTP 404 response means the Worker is absent; all other
 * failures must stop deployment rather than select the bootstrap path.
 */
export function workerExists(workerName: string, probe: () => unknown): boolean;
