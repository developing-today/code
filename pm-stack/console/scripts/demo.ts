// The app with made-up state, for looking at it in a browser:
//
//   bun run scripts/demo.ts [port]          the host alone (default 4899)
//   bun run scripts/edge-demo.ts [port]     the same host behind the edge
import { handle } from "../src/server.ts";
import { demoDeps } from "./demo-state.ts";

const extra = process.argv.slice(2);
if (extra.length > 1 || (extra[0] !== undefined && !/^[1-9][0-9]{1,4}$/.test(extra[0]))) {
  console.error("usage: bun run scripts/demo.ts [port]");
  process.exit(2);
}
const port = Number(extra[0] ?? 4899);
Bun.serve({
  hostname: "127.0.0.1",
  port,
  fetch: (req) => handle(req, demoDeps),
});
console.log(`demo: http://127.0.0.1:${port}/console/`);
