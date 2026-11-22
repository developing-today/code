// Protocol v1. Only these named, read-only capabilities cross the bridge.
export const operations = Object.freeze(['fleet.queue.list', 'fleet.queue.health', 'laptop.health', 'laptop.diagnostics']);
export const statuses = Object.freeze(['queued', 'claimed', 'parked', 'handed_off', 'blocked', 'done', 'cancelled', 'failed']);
export const MAX_BODY = 8192;
export const isObject = x => x !== null && typeof x === 'object' && !Array.isArray(x);
export const exact = (x, required, optional = []) => isObject(x) && required.every(k => Object.hasOwn(x, k)) && Object.keys(x).every(k => [...required, ...optional].includes(k));
export const identifier = x => typeof x === 'string' && !Object.hasOwn(Object.prototype, x) && x !== 'prototype' && /^[a-zA-Z0-9_-]{1,64}$/.test(x);
export function validateOperation(operation, args) {
  if (!operations.includes(operation)) return false;
  if (operation !== 'fleet.queue.list') return exact(args, []);
  return exact(args, ['limit'], ['status']) && Number.isInteger(args.limit) && args.limit >= 1 && args.limit <= 20 && (!Object.hasOwn(args, 'status') || statuses.includes(args.status));
}
export const b64url = bytes => btoa(String.fromCharCode(...new Uint8Array(bytes))).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
export function unbase64(value) {
  if (typeof value !== 'string' || !/^[A-Za-z0-9_-]+$/.test(value)) throw new Error('encoding');
  const bytes = Uint8Array.from(atob(value.replaceAll('-', '+').replaceAll('_', '/')), c => c.charCodeAt(0));
  if (b64url(bytes) !== value) throw new Error('encoding');
  return bytes;
}
export function signingBytes({audience, principal, method, path, timestamp, nonce, body}) {
  return new TextEncoder().encode(JSON.stringify(['fleet-private-bridge/v1', audience, principal, method, path, timestamp, nonce, body]));
}
export async function signRequest({origin, audience, principal, privateKey, path, body, now = Date.now(), nonce = crypto.randomUUID()}) {
  const raw = JSON.stringify(body);
  const timestamp = String(now);
  const signature = b64url(await crypto.subtle.sign('Ed25519', privateKey, signingBytes({audience, principal, method: 'POST', path, timestamp, nonce, body: raw})));
  return new Request(new URL(path, origin), {method: 'POST', headers: {'content-type': 'application/json', 'x-bridge-principal': principal, 'x-bridge-timestamp': timestamp, 'x-bridge-nonce': nonce, 'x-bridge-signature': signature}, body: raw});
}
export async function boundedText(stream, limit = MAX_BODY, timeoutMs = 3000) {
  if (!stream) return '';
  const reader = stream.getReader(); let size = 0; const chunks = [];
  let timer;
  const deadline = new Promise((_, reject) => { timer = setTimeout(() => { reject(new Error('timeout')); void reader.cancel().catch(() => {}); }, timeoutMs); });
  try {
    while (true) {
      const {value, done} = await Promise.race([reader.read(), deadline]); if (done) break;
      size += value.byteLength; if (size > limit) { void reader.cancel().catch(() => {}); throw new Error('size'); }
      chunks.push(value);
    }
  } finally { clearTimeout(timer); reader.releaseLock(); }
  const bytes = new Uint8Array(size); let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
  return new TextDecoder('utf-8', {fatal: true}).decode(bytes);
}
const count = x => Number.isSafeInteger(x) && x >= 0;
export function validateResult(operation, result) {
  if (!operations.includes(operation) || !isObject(result)) return false;
  if (result.ok === false) return exact(result, ['ok', 'error']) && ['unavailable', 'invalid-source', 'timeout'].includes(result.error);
  if (result.ok !== true || !exact(result, ['ok', 'data'])) return false;
  const d = result.data;
  if (operation === 'fleet.queue.list') return exact(d, ['tasks', 'complete', 'total', 'fetchedAt']) && count(d.fetchedAt) && d.complete === false && d.total === null && Array.isArray(d.tasks) && d.tasks.length <= 20 && d.tasks.every(t => exact(t, ['id', 'status']) && count(t.id) && t.id > 0 && statuses.includes(t.status));
  if (operation === 'fleet.queue.health') return exact(d, [...statuses, 'fetchedAt']) && count(d.fetchedAt) && statuses.every(k => count(d[k]));
  if (operation === 'laptop.health') return exact(d, ['queueBroker']) && ['up', 'down'].includes(d.queueBroker);
  return exact(d, ['uptimeSeconds', 'load1', 'memoryFreeBytes']) && count(d.uptimeSeconds) && count(d.memoryFreeBytes) && Number.isFinite(d.load1) && d.load1 >= 0 && d.load1 <= 100000;
}
