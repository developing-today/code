// Stateless Gateway routing rehearsal. No bridge credentials, storage or Fleet data.
import { WorkerEntrypoint } from 'cloudflare:workers';
const reply = (status) => new Response(status === 200 ? 'private-bridge-smoke-v1\n' : 'denied\n', {
  status, headers: {'cache-control':'no-store','content-type':'text/plain','x-content-type-options':'nosniff'}
});
export class GatewayMiddleware extends WorkerEntrypoint {
  async handle(request, context, _next) {
    const {ENABLED, GATEWAY_HOST:host, GATEWAY_ACCOUNT_ID:account, GATEWAY_POLICY_ID:policy, START_MS:start, END_MS:end} = this.env;
    const now = Date.now(), begins = Number(start), ends = Number(end);
    if (ENABLED !== 'true' || !/^\d{13}$/.test(start || '') || !/^\d{13}$/.test(end || '') || ends <= begins || ends - begins > 600000 || now < begins || now >= ends) return reply(503);
    if (typeof host !== 'string' || host.length > 253 || host.split('.').some(x => x.length > 63) || !/^(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+[a-z]{2,63}$/.test(host) || !/^[0-9a-f]{32}$/.test(account || '') || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(policy || '')) return reply(503);
    const expected = `https://${host}/smoke/v1`;
    if (request.method !== 'GET' || request.url !== expected || request.body !== null || [...request.headers.keys()].some(h => ['authorization','proxy-authorization','cookie','upgrade'].includes(h) || h.startsWith('x-bridge-'))) return reply(403);
    if (!context || context.accountId !== account || context.policyId !== policy || context.host !== host || context.uri !== expected || (context.sni !== undefined && context.sni !== host)) return reply(403);
    // Gateway policy supplies the approved transport restriction. Service bindings
    // could fabricate context and must never be granted to other Workers.
    return reply(200);
  }
}
export default {fetch() {return reply(403);}};
