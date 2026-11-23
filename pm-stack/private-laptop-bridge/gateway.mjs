// Private Gateway Forward RPC ingress. No public HTTP management entrypoint.
// Context contract: https://developers.cloudflare.com/cloudflare-one/traffic-controls/integrate-workers/available-context/
import { WorkerEntrypoint } from 'cloudflare:workers';
import bridge, {dispatchSignedRequest} from './worker.mjs';
export { PrivateLaptopBridge } from './worker.mjs';

const paths = new Set(['/v1/call', '/v1/poll', '/v1/result', '/v1/read']);
const uuid = value => typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value);
const deny = (status, error) => {
  console.warn(JSON.stringify({event: 'bridge.gateway-denied', status, code: error}));
  return Response.json({error}, {status, headers: {'cache-control': 'no-store', 'x-content-type-options': 'nosniff'}});
};
function configuration(env) {
  const {GATEWAY_HOST: host, GATEWAY_ACCOUNT_ID: account, GATEWAY_POLICY_ID: policy, GATEWAY_IDENTITY_EMAIL: email, GATEWAY_POSTURE_ID: posture} = env;
  // All fields are owner-supplied non-secret identifiers. Blank placeholders deny.
  if (typeof host !== 'string' || host.length > 253 || host.split('.').some(label => label.length > 63) || !/^(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+[a-z]{2,63}$/.test(host) || typeof account !== 'string' || !/^[0-9a-f]{32}$/.test(account) || !uuid(policy) || !uuid(posture) || typeof email !== 'string' || email.length > 254 || !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) return null;
  return {host, account, policy, email, posture};
}
function allowedUrl(raw, host) {
  try {
    const url = new URL(raw);
    return url.protocol === 'https:' && url.hostname === host && !url.port && !url.username && !url.password && !url.search && !url.hash && paths.has(url.pathname);
  } catch { return false; }
}

export class GatewayMiddleware extends WorkerEntrypoint {
  async handle(request, context, _next) {
    // _next is deliberately never invoked: no upstream origin, generic proxy,
    // network fallback, or route around this authorization decision exists here.
    if (this.env.ENABLED !== 'true') return deny(503, 'disabled');
    const config = configuration(this.env);
    if (!config) return deny(503, 'gateway-configuration');
    if (request.method !== 'POST' || request.headers.has('upgrade') || !allowedUrl(request.url, config.host)) return deny(403, 'gateway-route');
    try {
      if (!context || context.accountId !== config.account || context.policyId !== config.policy || context.host !== config.host || (context.sni !== undefined && context.sni !== config.host) || !allowedUrl(context.uri, config.host) || new URL(context.uri).pathname !== new URL(request.url).pathname) return deny(403, 'gateway-context');
      // These are trusted Gateway RPC getters, never request headers. A Worker
      // service binding to this entrypoint is privileged and must not be granted
      // to an untrusted Worker: that Worker could fabricate this context.
      let timer;
      try {
        const deadline = new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('deadline')), 3000); });
        const [identity, posture] = await Promise.race([Promise.all([context.identity(), context.devicePosture()]), deadline]);
        if (!identity || identity.email !== config.email || !Array.isArray(posture?.checks?.passed) || !posture.checks.passed.includes(config.posture)) return deny(403, 'gateway-identity');
      } finally { clearTimeout(timer); }
    } catch { return deny(503, 'gateway-context-unavailable'); }
    // Independent signed caller/device authentication, replay and scope checks
    // still run inside the existing Durable Object control plane.
    return dispatchSignedRequest(request, this.env);
  }
}

// The fixed custom hostname enters the Worker directly, but only after the
// Worker validates Access's signed service-token JWT and the DO independently
// validates the Ed25519 request. No proxy fallback or origin fetch exists.
export default {fetch(request, env) { return bridge.fetch(request, env); }};
