// CLI contract: --config ABSOLUTE_PATH only, or --help alone. Parse the entire
// command line before file/network activity. No key generation or installation.
import { open, constants } from 'node:fs/promises';
import { isAbsolute } from 'node:path';
import { pathToFileURL } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { signRequest, validateOperation, validateResult, identifier } from './protocol.mjs';
import { createAdapters } from './adapters.mjs';

const exact = (v, list) => v && typeof v === 'object' && !Array.isArray(v) && Object.keys(v).length === list.length && list.every(k => Object.hasOwn(v, k));
const id = identifier;
export class BridgeDenied extends Error { constructor() { super('bridge-denied'); } }
export function parseArgs(argv) {
  if (argv.length === 1 && argv[0] === '--help') return { help: true };
  if (argv.length !== 2 || argv[0] !== '--config' || !isAbsolute(argv[1])) throw new Error('usage');
  return { config: argv[1] };
}
export function requireNonRoot(getuid = process.getuid) {
  if (typeof getuid !== 'function' || !Number.isInteger(getuid()) || getuid() <= 0) throw new Error('non-root-required');
}
export function validateConfig(config) {
  if (!exact(config, ['origin','audience','principal','privateKey'])) throw new Error('invalid-config');
  const origin = new URL(config.origin);
  if (origin.protocol !== 'https:' || origin.origin !== config.origin || origin.username || origin.password || !id(config.principal) || typeof config.audience !== 'string' || config.audience.length < 1 || config.audience.length > 200) throw new Error('invalid-config');
  if (!exact(config.privateKey, ['kty','crv','x','d']) || config.privateKey.kty !== 'OKP' || config.privateKey.crv !== 'Ed25519' || !/^[A-Za-z0-9_-]{43}$/.test(config.privateKey.x) || !/^[A-Za-z0-9_-]{43}$/.test(config.privateKey.d)) throw new Error('invalid-config');
  return config;
}
export async function readConfig(path, getuid = process.getuid) {
  requireNonRoot(getuid);
  const file = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  try {
    const st = await file.stat();
    if (!st.isFile() || st.size > 8192 || st.uid !== getuid() || (st.mode & 0o077)) throw new Error('invalid-config');
    const buffer = Buffer.alloc(8193);
    const { bytesRead } = await file.read(buffer, 0, buffer.length, 0);
    if (bytesRead > 8192) throw new Error('invalid-config');
    return validateConfig(JSON.parse(buffer.subarray(0, bytesRead).toString('utf8')));
  } finally { await file.close(); }
}
async function readResponse(response, signal) {
  if (!response.body) throw new Error('invalid-response');
  const reader = response.body.getReader();
  const abort = () => { void reader.cancel().catch(() => {}); };
  signal.addEventListener('abort', abort, { once: true });
  const chunks = []; let size = 0;
  try {
    for (;;) {
      signal.throwIfAborted();
      const { done, value } = await reader.read();
      if (done) break;
      size += value.length;
      if (size > 16384) throw new Error('response-too-large');
      chunks.push(value);
    }
    signal.throwIfAborted();
    return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(chunks)));
  } finally { signal.removeEventListener('abort', abort); void reader.cancel().catch(() => {}); reader.releaseLock(); }
}
export function createClient({ origin, audience, principal, privateKey, fetch: fetchFn = globalThis.fetch, execute = createAdapters(), now = Date.now, sleep = delay, getuid = process.getuid, requestTimeoutMs = 5000 }) {
  requireNonRoot(getuid);
  const url = new URL(origin);
  if (url.protocol !== 'https:' || url.origin !== origin || !id(principal) || typeof audience !== 'string' || !audience) throw new Error('invalid-config');
  async function send(path, body, signal) {
    const bounded = signal ? AbortSignal.any([signal, AbortSignal.timeout(requestTimeoutMs)]) : AbortSignal.timeout(requestTimeoutMs);
    const startedAt = now();
    const request = await signRequest({ origin, audience, principal, privateKey, path, body, now: startedAt });
    const response = await fetchFn(request, { redirect: 'error', signal: bounded });
    if ([401,403,410].includes(response.status)) { void response.body?.cancel().catch(() => {}); throw new BridgeDenied(); }
    if (!response.ok || response.redirected) { void response.body?.cancel().catch(() => {}); throw new Error('bridge-unavailable'); }
    return { body: await readResponse(response, bounded), startedAt };
  }
  async function step(signal) {
    const {body: response, startedAt} = await send('/v1/poll', {}, signal);
    if (!exact(response, ['job'])) throw new Error('invalid-response');
    const job = response.job;
    if (job === null) return false;
    if (!exact(job, ['id','lease','operation','args','expiresAt','leaseExpiresAt','serverNow']) || !id(job.id) || !id(job.lease) || !validateOperation(job.operation, job.args)
      || !Number.isSafeInteger(job.serverNow) || Math.abs(job.serverNow - startedAt) > 30000
      || !Number.isSafeInteger(job.expiresAt) || job.expiresAt <= job.serverNow || job.expiresAt - job.serverNow > 60000
      || !Number.isSafeInteger(job.leaseExpiresAt) || job.leaseExpiresAt <= job.serverNow || job.leaseExpiresAt - job.serverNow > 15000
      || job.leaseExpiresAt > job.expiresAt) throw new Error('invalid-job');
    // Translate server-relative lifetimes to local deadlines. Starting at the
    // request send time conservatively charges both network legs to the budget;
    // the clocks may differ by the protocol's authenticated 30-second skew.
    const expiresAt = startedAt + (job.expiresAt - job.serverNow);
    const leaseExpiresAt = startedAt + (job.leaseExpiresAt - job.serverNow);
    if (expiresAt <= now() || leaseExpiresAt <= now()) throw new Error('expired-job');
    signal?.throwIfAborted();
    const abort = AbortSignal.timeout(Math.max(1, Math.min(5000, leaseExpiresAt - now())));
    let onAbort;
    const expired = new Promise((_, reject) => { onAbort = () => reject(new Error('execution-timeout')); abort.addEventListener('abort', onAbort, { once: true }); });
    let result;
    try { result = await Promise.race([execute(job.operation, job.args), expired]); }
    finally { abort.removeEventListener('abort', onAbort); }
    signal?.throwIfAborted();
    if (expiresAt <= now() || leaseExpiresAt <= now()) throw new Error('expired-job');
    if (!validateResult(job.operation, result)) throw new Error('invalid-result');
    const {body: accepted} = await send('/v1/result', { id: job.id, lease: job.lease, result }, signal);
    if (!exact(accepted, ['accepted']) || accepted.accepted !== true) throw new Error('invalid-ack');
    return true;
  }
  async function run(signal) {
    let backoff = 1000;
    while (!signal?.aborted) {
      // Reserve request quota for completions; idle polling uses at most 30/min.
      try { await step(signal); backoff = 1000; await sleep(2000, undefined, { signal }); }
      catch (error) {
        if (signal?.aborted) return;
        if (error instanceof BridgeDenied) throw error; // Explicit revocation needs owner reconnect.
        try { await sleep(backoff, undefined, { signal }); }
        catch (sleepError) { if (signal?.aborted) return; throw sleepError; }
        backoff = Math.min(30000, backoff * 2);
      }
    }
  }
  return { step, run };
}
export async function main(argv = process.argv.slice(2)) {
  const args = parseArgs(argv);
  if (args.help) { process.stdout.write('Usage: node client.mjs --config ABSOLUTE_PATH\n'); return; }
  const config = await readConfig(args.config);
  const privateKey = await crypto.subtle.importKey('jwk', config.privateKey, { name: 'Ed25519' }, false, ['sign']);
  const abort = new AbortController();
  const stop = () => abort.abort();
  process.once('SIGTERM', stop); process.once('SIGINT', stop);
  try { await createClient({ ...config, privateKey }).run(abort.signal); }
  finally { process.removeListener('SIGTERM', stop); process.removeListener('SIGINT', stop); }
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch(error => { process.stderr.write(error.message === 'usage' ? 'Usage: node client.mjs --config ABSOLUTE_PATH\n' : 'Private laptop bridge stopped; inspect configuration and authorization.\n'); process.exitCode = error.message === 'usage' ? 2 : 1; });
}
