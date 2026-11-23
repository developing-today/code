// Caller SDK: four named reads, an explicit accepted handle, and bounded polling.
// Keys are supplied by an approved caller integration; no enrollment or files.
import { exact, identifier, validateOperation, validateResult, signRequest } from './protocol.mjs';
import { BRIDGE_ORIGIN } from './access-auth.mjs';

export class CallerError extends Error {
  constructor(code, handle) {
    super(code);
    this.name = 'CallerError';
    this.code = code;
    // A caller can resume an uncertain read with its original accepted handle.
    // Never attach raw responses, credentials or transport errors.
    if (handle) this.handle = handle;
  }
}

function validHandle(handle) {
  return exact(handle, ['id', 'operation', 'expiresAt', 'args']) && identifier(handle.id)
    && validateOperation(handle.operation, handle.args) && Number.isSafeInteger(handle.expiresAt);
}

export function createCaller({ origin, audience, principal, device, privateKey, accessClientId, accessClientSecret, fetch: fetchFn = globalThis.fetch,
  now = Date.now, requestTimeoutMs = 3000 }) {
  let url;
  try { url = new URL(origin); } catch { throw new CallerError('invalid-config'); }
  if (url.protocol !== 'https:' || url.origin !== origin || url.origin !== BRIDGE_ORIGIN || url.username || url.password
    || !identifier(audience) || !identifier(principal) || !identifier(device)
    || typeof accessClientId !== 'string' || !/^[A-Za-z0-9._-]{8,128}$/.test(accessClientId)
    || typeof accessClientSecret !== 'string' || accessClientSecret.length < 32 || accessClientSecret.length > 256
    || !privateKey || privateKey.type !== 'private' || privateKey.algorithm?.name !== 'Ed25519'
    || !Number.isInteger(requestTimeoutMs) || requestTimeoutMs < 1 || requestTimeoutMs > 3000) throw new CallerError('invalid-config');

  async function transport(path, body, signal) {
    const controller = new AbortController();
    let timer, reader, response;
    const startedAt = now();
    const abort = () => controller.abort();
    if (signal?.aborted) throw new CallerError('cancelled');
    signal?.addEventListener('abort', abort, { once: true });
    const aborted = new Promise((_, reject) => {
      controller.signal.addEventListener('abort', () => {
        void reader?.cancel().catch(() => {});
        reject(new CallerError(signal?.aborted ? 'cancelled' : 'timeout'));
      }, { once: true });
      timer = setTimeout(abort, requestTimeoutMs);
    });
    try {
      const work = async () => {
        controller.signal.throwIfAborted();
        const signed = await signRequest({ origin, audience, principal, privateKey, path, body, now: startedAt });
        const headers = new Headers(signed.headers);
        headers.set('cf-access-client-id', accessClientId);
        headers.set('cf-access-client-secret', accessClientSecret);
        const request = new Request(signed, {headers});
        controller.signal.throwIfAborted();
        response = await fetchFn(request, { redirect: 'error', signal: controller.signal });
        if (controller.signal.aborted) { void response.body?.cancel().catch(() => {}); controller.signal.throwIfAborted(); }
        if (response.redirected || (response.status >= 300 && response.status < 400)) throw new CallerError('invalid-response');
        if ([401, 403, 410].includes(response.status)) throw new CallerError('denied');
        if (response.status === 429) throw new CallerError('quota');
        if (!response.ok) throw new CallerError('unavailable');
        if (!response.body) throw new CallerError('invalid-response');
        reader = response.body.getReader();
        const chunks = []; let size = 0;
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          size += value.byteLength;
          if (size > 8192) throw new CallerError('invalid-response');
          chunks.push(value);
        }
        const bytes = new Uint8Array(size); let offset = 0;
        for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
        const decoded = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
        return { status: response.status, body: decoded, startedAt };
      };
      return await Promise.race([work(), aborted]);
    } catch (error) {
      if (error instanceof CallerError) throw error;
      throw new CallerError('unavailable');
    } finally {
      clearTimeout(timer);
      signal?.removeEventListener('abort', abort);
      if (reader) { void reader.cancel().catch(() => {}); reader.releaseLock(); }
      else { void response?.body?.cancel().catch(() => {}); }
    }
  }

  async function submit(operation, args, { signal } = {}) {
    if (!validateOperation(operation, args)) throw new CallerError('invalid-operation');
    if (signal?.aborted) throw new CallerError('cancelled');
    const requested = structuredClone(args);
    let reply;
    try { reply = await transport('/v1/call', { device, operation, args: requested }, signal); }
    catch (error) {
      // Never automatically retry a submit: a lost acknowledgement may already
      // have created a read request. Authentication/quota refusals are definite.
      if (['denied', 'quota'].includes(error.code)) throw error;
      throw new CallerError('submit-outcome-unknown');
    }
    if (reply.status !== 202 || !exact(reply.body, ['id', 'expiresAt', 'serverNow']) || !identifier(reply.body.id)
      || !Number.isSafeInteger(reply.body.serverNow) || Math.abs(reply.body.serverNow - reply.startedAt) > 30000
      || !Number.isSafeInteger(reply.body.expiresAt) || reply.body.expiresAt <= reply.body.serverNow
      || reply.body.expiresAt - reply.body.serverNow > 60000) throw new CallerError('submit-outcome-unknown');
    // The handle deadline is local, not server UTC. Count transport latency
    // against the server-issued duration, independently of clock offset.
    const expiresAt = reply.startedAt + (reply.body.expiresAt - reply.body.serverNow);
    if (expiresAt <= now()) throw new CallerError('submit-outcome-unknown');
    return Object.freeze({ id: reply.body.id, operation, expiresAt, args: Object.freeze(requested) });
  }

  async function read(handle, { signal } = {}) {
    if (!validHandle(handle)) throw new CallerError('invalid-handle');
    if (handle.expiresAt <= now()) throw new CallerError('expired', handle);
    try {
      const reply = await transport('/v1/read', { id: handle.id }, signal);
      if (reply.status !== 200) throw new CallerError('invalid-response');
      if (exact(reply.body, ['status']) && reply.body.status === 'pending') return { status: 'pending', handle };
      if (!exact(reply.body, ['status', 'result']) || reply.body.status !== 'complete'
        || !validateResult(handle.operation, reply.body.result)) throw new CallerError('invalid-response');
      const r = reply.body.result;
      if (r.ok && handle.operation === 'fleet.queue.list'
        && (r.data.tasks.length > handle.args.limit || (handle.args.status && r.data.tasks.some(task => task.status !== handle.args.status)))) throw new CallerError('invalid-response');
      if (r.ok && handle.operation.startsWith('fleet.queue.')
        && (r.data.fetchedAt > now() || r.data.fetchedAt < now() - 300000)) throw new CallerError('stale-result');
      return { status: 'complete', handle, result: r };
    } catch (error) {
      throw new CallerError(error instanceof CallerError ? error.code : 'unavailable', handle);
    }
  }

  async function wait(handle, { signal } = {}) {
    if (!validHandle(handle)) throw new CallerError('invalid-handle');
    // Bound wall-clock time even if an injected clock stalls. Reads may be
    // resumed with the handle; no submission happens inside this loop.
    const stop = Date.now() + Math.min(60000, Math.max(0, handle.expiresAt - now()));
    while (Date.now() < stop) {
      const reply = await read(handle, { signal });
      if (reply.status === 'complete') return reply;
      await new Promise((resolve, reject) => {
        const abort = () => { clearTimeout(timer); signal?.removeEventListener('abort', abort); reject(new CallerError('cancelled', handle)); };
        const timer = setTimeout(() => { signal?.removeEventListener('abort', abort); resolve(); }, Math.min(1000, Math.max(0, stop - Date.now())));
        signal?.addEventListener('abort', abort, { once: true });
        if (signal?.aborted) abort();
      });
    }
    throw new CallerError('expired', handle);
  }
  return Object.freeze({
    fleetQueueList: (args, options) => submit('fleet.queue.list', args, options),
    fleetQueueHealth: options => submit('fleet.queue.health', {}, options),
    laptopHealth: options => submit('laptop.health', {}, options),
    laptopDiagnostics: options => submit('laptop.diagnostics', {}, options),
    read, wait,
  });
}
