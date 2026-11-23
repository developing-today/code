// Test-only Miniflare entrypoint: exercises the real SQLite DO and its signed
// protocol without simulating Cloudflare Access. Production config points at
// gateway.mjs, whose default path validates Access before invoking the DO.
export {PrivateLaptopBridge} from './worker.mjs';
export default {
  fetch(request, env) {
    return env.BRIDGE.get(env.BRIDGE.idFromName('owner-v1')).fetch(request);
  },
};
