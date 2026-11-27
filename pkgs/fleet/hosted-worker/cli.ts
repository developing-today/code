#!/usr/bin/env node
// fleet-hosted CLI entrypoint (#12588; design §3.2) — wires
// src/cli.ts's command line to a real caller's machine: global
// fetch as the transport, the process environment, and the two
// caller-local stores the design gives the CLI — the per-workspace
// session state file (.fleet-hosted-session in the working
// directory, the hosted analogue of the native pwd-file) and the
// token file (~/.config/fleet-hosted/token, or
// FLEET_HOSTED_TOKEN_FILE). Configuration by environment:
//   FLEET_HOSTED_URL   service base URL (default the wrangler dev
//                      address; this package is not deployed)
//   FLEET_HOSTED_TOKEN the caller's #9077 relying-party token
//                      (wins over the token file)
//   FLEET_WORKER       default worker id for next/claim
//   FLEET_SESSION_ID   carried session (see src/session-resolution.ts)
// Run: node --experimental-transform-types cli.ts <verb> ...
// (package.json's "cli" script and "bin" entry point here.)

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { hostname } from "node:os";
import { join } from "node:path";
import { callerTokenFile, resolveCallerToken, runCli, type CliDeps } from "./src/cli.ts";
import type { SessionStateStore } from "./src/session-resolution.ts";

class FileSessionStateStore implements SessionStateStore {
  constructor(private path: string) {}

  read(): string | null {
    if (!existsSync(this.path)) return null;
    return readFileSync(this.path, "utf8");
  }

  write(sessionId: string): void {
    writeFileSync(this.path, `${sessionId}\n`);
  }
}

const env = (name: string): string | null => process.env[name] ?? null;

const tokenFile = callerTokenFile(process.env);

const deps: CliDeps = {
  serviceUrl: (process.env["FLEET_HOSTED_URL"] ?? "http://localhost:8787").replace(/\/+$/, ""),
  token: resolveCallerToken({
    env,
    stored: existsSync(tokenFile) ? readFileSync(tokenFile, "utf8") : null,
  }),
  fetch: (req) => fetch(req),
  env,
  store: new FileSessionStateStore(join(process.cwd(), ".fleet-hosted-session")),
  readFile: (path) => readFileSync(path, "utf8"),
  hostname: () => hostname(),
  cwd: () => process.cwd(),
};

const outcome = await runCli(process.argv.slice(2), deps);
if (outcome.stdout !== "") process.stdout.write(outcome.stdout);
if (outcome.stderr !== "") process.stderr.write(outcome.stderr);
process.exit(outcome.exitCode);
