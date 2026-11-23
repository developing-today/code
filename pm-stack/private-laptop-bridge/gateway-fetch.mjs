// Keep the direct Access-authenticated Worker entrypoint's execution context.
// Cloudflare attaches ctx.access only to the directly authenticated invocation.
export async function gatewayFetch(bridge, request, env, ctx) {
  return bridge.fetch(request, env, ctx);
}
