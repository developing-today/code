import { open, constants } from 'node:fs/promises';
import { request } from 'node:http';
import { homedir, uptime, loadavg, freemem } from 'node:os';
import { join } from 'node:path';
import { validateOperation, validateResult } from './protocol.mjs';

const STATES = ['queued', 'claimed', 'parked', 'handed_off', 'blocked', 'done', 'cancelled', 'failed'];
const MAX_SOURCE_BYTES = 1024 * 1024;
const integer = (v) => Number.isSafeInteger(v) && v >= 0;
const keys = (v, allowed) => v && typeof v === 'object' && !Array.isArray(v) && Object.keys(v).length === allowed.length && allowed.every(k => Object.hasOwn(v, k));
const failure = (error = 'unavailable') => ({ ok: false, error });

// Only this producer-owned snapshot is read. No caller path, token, or broker
// command is accepted. O_NOFOLLOW and regular-file/size checks precede reading.
export async function readQueueSnapshot(home = homedir()) {
  const file = await open(join(home, '.local/state/pm-aggregator/bridge-queue.json'), constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  try {
    const st = await file.stat();
    if (!st.isFile() || st.size > MAX_SOURCE_BYTES) throw new Error('invalid-source');
    const buffer = Buffer.alloc(MAX_SOURCE_BYTES + 1);
    let offset = 0;
    while (offset < buffer.length) {
      const { bytesRead } = await file.read(buffer, offset, buffer.length - offset, null);
      if (!bytesRead) break;
      offset += bytesRead;
    }
    if (offset > MAX_SOURCE_BYTES) throw new Error('invalid-source');
    return JSON.parse(buffer.subarray(0, offset).toString('utf8'));
  } finally { await file.close(); }
}

export function projectQueueSnapshot(snapshot, operation, args, now = Date.now()) {
  if (!keys(snapshot, ['version','fetchedAt','available','stale','complete','total','tasks','counts']) || snapshot.version !== 1 || !Number.isSafeInteger(snapshot.fetchedAt) || snapshot.complete !== false || snapshot.total !== null) return failure('invalid-source');
  if (snapshot.available !== true || snapshot.stale !== false || now - snapshot.fetchedAt > 300000 || snapshot.fetchedAt > now + 5000) return failure();
  if (!Array.isArray(snapshot.tasks) || snapshot.tasks.length > 1000 || !keys(snapshot.counts, STATES) || !STATES.every(s => integer(snapshot.counts[s]))) return failure('invalid-source');
  if (!snapshot.tasks.every(t => keys(t, ['id','status','priority']) && integer(t.id) && t.id > 0 && STATES.includes(t.status) && (t.priority === null || Number.isFinite(t.priority)))) return failure('invalid-source');
  if (new Set(snapshot.tasks.map(t => t.id)).size !== snapshot.tasks.length) return failure('invalid-source');
  if (operation === 'fleet.queue.health') return { ok: true, data: Object.fromEntries(STATES.map(s => [s, snapshot.counts[s]])) };
  return { ok: true, data: { tasks: snapshot.tasks.filter(t => !args.status || t.status === args.status).slice(0, args.limit).map(({id,status}) => ({id,status})), complete: false, total: null } };
}

// A single unauthenticated GET liveness route over the existing broker's UDS.
// This never reads its all-verbs token and never falls back to TCP.
export function queueBrokerAlive(home = homedir()) {
  return new Promise(resolve => {
    const req = request({ socketPath: join(home, '.config/queue-broker/broker.sock'), method: 'GET', path: '/healthz', agent: false, headers: { connection: 'close' } }, res => {
      let bytes = 0;
      res.on('data', chunk => { bytes += chunk.length; if (bytes > 4096) req.destroy(); });
      res.on('end', () => { clearTimeout(timer); resolve(res.statusCode === 200 && bytes <= 4096); });
      res.on('error', () => { clearTimeout(timer); resolve(false); });
    });
    const timer = setTimeout(() => req.destroy(), 2000);
    req.on('error', () => { clearTimeout(timer); resolve(false); });
    req.end();
  });
}

export function createAdapters({ readSnapshot = readQueueSnapshot, probe = queueBrokerAlive, clock = Date.now, diagnostics = () => ({ uptimeSeconds: Math.floor(uptime()), load1: loadavg()[0], memoryAvailableBytes: freemem() }) } = {}) {
  return async function execute(operation, args) {
    if (!validateOperation(operation, args)) return failure('invalid-source');
    let result;
    try {
      if (operation.startsWith('fleet.queue.')) result = projectQueueSnapshot(await readSnapshot(), operation, args, clock());
      else if (operation === 'laptop.health') result = { ok: true, data: { queueBroker: await probe() ? 'up' : 'down' } };
      else result = { ok: true, data: diagnostics() };
    } catch { return failure(); }
    return validateResult(operation, result) ? result : failure('invalid-source');
  };
}
