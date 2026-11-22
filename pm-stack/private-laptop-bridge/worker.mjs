import {operations, exact, identifier, unbase64, signingBytes, boundedText, validateOperation, validateResult} from './protocol.mjs';

const nextExpiry = s => Math.min(...Object.values(s.nonces), ...Object.values(s.quotas).map(q => q.end), ...Object.values(s.jobs).map(j => j.retainUntil), ...s.audit.map(e => e.at + 180000));
const paths = ['/v1/call', '/v1/poll', '/v1/result', '/v1/read'];
const response = (status, body) => {
  // Unauthenticated denials cannot safely name an identity. Never echo request values.
  if (status >= 400) console.warn(JSON.stringify({event: 'bridge.denied', status, code: body.error}));
  return Response.json(body, {status, headers: {'cache-control': 'no-store', 'x-content-type-options': 'nosniff'}});
};
const deny = (status, error) => ({status, body: {error}});
const active = (p, now) => p && !p.revoked && p.issuedAt <= now && p.expiresAt > now;
const sourceFresh = (op, result, now) => !op.startsWith('fleet.queue.') || !result?.ok || (result.data.fetchedAt <= now + 30000 && now - result.data.fetchedAt <= 300000);
const allows = (p, device, op, now) => active(p, now) && p.device === device && p.scopes.includes(op);

function registry(raw) {
  if (typeof raw !== 'string' || raw.length > 16384) throw new Error('config');
  const r = JSON.parse(raw);
  if (!exact(r, ['audience', 'principals']) || !identifier(r.audience) || !Array.isArray(r.principals) || r.principals.length < 2 || r.principals.length > 16) throw new Error('config');
  const ids = new Set(), keys = new Set(), principals = {};
  for (const p of r.principals) {
    if (!exact(p, ['id', 'role', 'device', 'publicKey', 'scopes', 'issuedAt', 'expiresAt', 'revoked']) || !identifier(p.id) || !identifier(p.device) || !['caller', 'device'].includes(p.role) || typeof p.revoked !== 'boolean' || !Number.isSafeInteger(p.issuedAt) || !Number.isSafeInteger(p.expiresAt) || p.expiresAt <= p.issuedAt || p.expiresAt - p.issuedAt > 3600000 || !Array.isArray(p.scopes) || p.scopes.length < 1 || p.scopes.length > operations.length || new Set(p.scopes).size !== p.scopes.length || !p.scopes.every(s => operations.includes(s)) || ids.has(p.id) || keys.has(p.publicKey) || unbase64(p.publicKey).length !== 32) throw new Error('config');
    ids.add(p.id); keys.add(p.publicKey); principals[p.id] = p;
  }
  return {audience: r.audience, principals};
}

export class PrivateLaptopBridge {
  constructor(state, env) { this.state = state; this.env = env; }
  async fetch(request) {
    if (this.env.ENABLED !== 'true') return response(503, {error: 'disabled'});
    let now = Date.now(); let config;
    try { config = registry(this.env.BRIDGE_REGISTRY); } catch { return response(503, {error: 'configuration'}); }
    const url = new URL(request.url);
    if (request.method !== 'POST' || url.search || !paths.includes(url.pathname)) return response(404, {error: 'route'});
    if (request.headers.get('content-type') !== 'application/json') return response(415, {error: 'content-type'});
    const id = request.headers.get('x-bridge-principal');
    const principal = identifier(id) && Object.hasOwn(config.principals, id) ? config.principals[id] : null;
    const timestamp = request.headers.get('x-bridge-timestamp'), nonce = request.headers.get('x-bridge-nonce');
    if (!active(principal, now) || !/^\d{13}$/.test(timestamp ?? '') || Math.abs(now - Number(timestamp)) > 30000 || !identifier(nonce)) return response(401, {error: 'authentication'});
    const length = request.headers.get('content-length');
    if (length !== null && (!/^\d{1,5}$/.test(length) || Number(length) > 8192)) return response(413, {error: 'body'});
    if (request.headers.get('x-bridge-signature')?.length !== 86) return response(401, {error: 'authentication'});
    let raw;
    try { raw = await boundedText(request.body); } catch { return response(413, {error: 'body'}); }
    try {
      const key = await crypto.subtle.importKey('raw', unbase64(principal.publicKey), 'Ed25519', false, ['verify']);
      if (!await crypto.subtle.verify('Ed25519', key, unbase64(request.headers.get('x-bridge-signature')), signingBytes({audience: config.audience, principal: id, method: request.method, path: url.pathname, timestamp, nonce, body: raw}))) return response(401, {error: 'authentication'});
    } catch { return response(401, {error: 'authentication'}); }
    try {
      const outcome = await this.state.storage.transaction(async tx => {
        now = Date.now();
        if (!active(principal, now) || Math.abs(now - Number(timestamp)) > 30000) return deny(401, 'authentication');
        const s = await tx.get('state') ?? {nonces: {}, quotas: {}, jobs: {}, audit: []};
        for (const [k, expiry] of Object.entries(s.nonces)) if (expiry <= now) delete s.nonces[k];
        for (const [k, q] of Object.entries(s.quotas)) if (q.end <= now) delete s.quotas[k];
        for (const [k, job] of Object.entries(s.jobs)) if (job.retainUntil <= now) delete s.jobs[k];
        const replay = `${id}:${nonce}`;
        let result;
        const q = s.quotas[id] ?? {end: now + 60000, count: 0};
        s.quotas[id] = q;
        if (q.count >= 60) result = deny(429, 'quota');
        else {
          q.count++;
          if (s.nonces[replay]) result = deny(409, 'replay');
          else if (Object.keys(s.nonces).length >= 256) result = deny(429, 'capacity');
          else {
            s.nonces[replay] = now + 61000;
            let body; try { body = JSON.parse(raw); } catch { body = null; }
            result = this.route(url.pathname, body, principal, config, s, now);
          }
        }
        // Audit contains only server-selected codes and validated identity labels.
        s.audit.push({at: now, principal: id, role: principal.role, route: url.pathname, status: result.status});
        s.audit = s.audit.filter(e => e.at + 180000 > now).slice(-64);
        await tx.put('state', s);
        // Keep data retention bounded even when no later request arrives.
        await tx.setAlarm(nextExpiry(s));
        return result;
      });
      return response(outcome.status, outcome.body);
    } catch { return response(503, {error: 'storage'}); }
  }
  async alarm() {
    // All records expire within 180s. A single serialized transaction prevents
    // deleting a request accepted concurrently with cleanup.
    const now = Date.now();
    await this.state.storage.transaction(async tx => {
      const s = await tx.get('state'); if (!s) return;
      for (const [k, expiry] of Object.entries(s.nonces)) if (expiry <= now) delete s.nonces[k];
      for (const [k, q] of Object.entries(s.quotas)) if (q.end <= now) delete s.quotas[k];
      for (const [k, job] of Object.entries(s.jobs)) if (job.retainUntil <= now) delete s.jobs[k];
      s.audit = s.audit.filter(e => e.at + 180000 > now);
      await tx.put('state', s);
      const next = nextExpiry(s);
      if (Number.isFinite(next)) await tx.setAlarm(next);
    });
  }
  route(path, body, p, config, s, now) {
    const liveJob = job => job && job.expiresAt > now && allows(config.principals[job.caller], job.device, job.operation, now) && config.principals[job.caller].role === 'caller';
    if (path === '/v1/call') {
      if (p.role !== 'caller') return deny(403, 'role');
      if (!exact(body, ['device', 'operation', 'args']) || !identifier(body.device) || !validateOperation(body.operation, body.args)) return deny(400, 'operation');
      if (!allows(p, body.device, body.operation, now)) return deny(403, 'scope');
      if (!Object.values(config.principals).some(d => d.role === 'device' && allows(d, body.device, body.operation, now))) return deny(403, 'device');
      if (Object.keys(s.jobs).length >= 8) return deny(429, 'capacity');
      const id = crypto.randomUUID();
      s.jobs[id] = {id, device: body.device, operation: body.operation, args: body.args, caller: p.id, expiresAt: now + 60000, retainUntil: now + 120000, lease: null, leaseUntil: 0, assignee: null};
      return {status: 202, body: {id, expiresAt: s.jobs[id].expiresAt, serverNow: now}};
    }
    if (path === '/v1/poll') {
      if (p.role !== 'device') return deny(403, 'role');
      if (!exact(body, [])) return deny(400, 'body');
      const job = Object.values(s.jobs).find(j => liveJob(j) && !j.result && j.leaseUntil <= now && allows(p, j.device, j.operation, now));
      if (!job) return {status: 200, body: {job: null}};
      job.lease = crypto.randomUUID(); job.leaseUntil = Math.min(now + 15000, job.expiresAt); job.assignee = p.id;
      return {status: 200, body: {job: {id: job.id, lease: job.lease, operation: job.operation, args: job.args, expiresAt: job.expiresAt, leaseExpiresAt: job.leaseUntil, serverNow: now}}};
    }
    if (path === '/v1/result') {
      if (p.role !== 'device') return deny(403, 'role');
      if (!exact(body, ['id', 'lease', 'result']) || !identifier(body.id) || !identifier(body.lease)) return deny(400, 'body');
      const job = s.jobs[body.id];
      if (!liveJob(job) || job.result || job.leaseUntil <= now || job.lease !== body.lease || job.assignee !== p.id || !allows(p, job.device, job.operation, now)) return deny(409, 'lease');
      if (!validateResult(job.operation, body.result) || !sourceFresh(job.operation, body.result, now) || (job.operation === 'fleet.queue.list' && body.result.ok && (body.result.data.tasks.length > job.args.limit || (job.args.status && body.result.data.tasks.some(t => t.status !== job.args.status))))) return deny(400, 'result');
      job.result = body.result;
      return {status: 200, body: {accepted: true}};
    }
    if (p.role !== 'caller') return deny(403, 'role');
    if (!exact(body, ['id']) || !identifier(body.id)) return deny(400, 'body');
    const job = s.jobs[body.id];
    if (!job || job.caller !== p.id) return deny(404, 'job');
    if (!liveJob(job) || (job.assignee && (config.principals[job.assignee]?.role !== 'device' || !allows(config.principals[job.assignee], job.device, job.operation, now)))) return deny(410, 'expired-or-revoked');
    if (job.result && !sourceFresh(job.operation, job.result, now)) return deny(410, 'stale-source');
    return {status: 200, body: job.result ? {status: 'complete', result: job.result} : {status: 'pending'}};
  }
}

export default {
  async fetch(request, env) {
    if (env.ENABLED !== 'true' || !env.BRIDGE) return response(503, {error: 'disabled'});
    try { return await env.BRIDGE.get(env.BRIDGE.idFromName('owner-v1')).fetch(request); }
    catch { return response(503, {error: 'storage'}); }
  },
};
