export const DEFAULT_PORT = 7801;

/** Parse the projector's listening port without accepting Number() coercions. */
export function parsePort(raw: string | undefined): number {
  if (raw === undefined) return DEFAULT_PORT;

  if (!/^[0-9]+$/.test(raw)) {
    throw new Error(`Invalid PORT ${JSON.stringify(raw)}; expected a decimal integer in range 1-65535`);
  }

  const port = Number(raw);
  if (!Number.isSafeInteger(port) || port < 1 || port > 65535) {
    throw new Error(`Invalid PORT ${JSON.stringify(raw)}; expected a decimal integer in range 1-65535`);
  }
  return port;
}
