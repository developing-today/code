// Standalone deploy entrypoint for the known-good Cloudflare container probe.
// Image publication and Worker activation are separate commands by design:
//   deploy.mjs publish-image [--dry-run]
//   deploy.mjs activate [--dry-run]
// The probe uses Cloudflare's official hello-world image; there is no local
// image recipe to publish. `publish-image` uploads a Worker version with cf;
// `activate --version <id>` separately puts that existing version live.
//
// Every command-line form is checked before subprocesses or network access.
import { execFileSync } from "node:child_process";
import { parseArgs } from "./deploy-args.mjs";

let options;
try {
  options = parseArgs(process.argv.slice(2));
} catch (error) {
  console.error(`deploy.mjs: ${error.message}`);
  process.exit(2);
}

const WORKER = "cf-container-hello";
if (options.command === "publish-image") {
  const args = ["cf@1.0.0-beta.20", "workers", "versions", "create", "--worker-id", WORKER];
  if (options.dryRun) args.push("--dry-run");
  execFileSync("npx", args, { stdio: "inherit" });
  process.exit(0);
}
if (options.dryRun) {
  console.error("activation dry-run is a no-op; no Cloudflare API call made");
  process.exit(0);
}
if (!options.version) {
  console.error("activate requires --version <id> from publish-image output");
  process.exit(2);
}
const versions = JSON.stringify([{ version_id: options.version, percentage: 100 }]);
execFileSync("npx", [
  "cf@1.0.0-beta.20", "workers", "deployments", "create",
  "--worker-id", WORKER, "--strategy", "percentage", "--versions", versions,
], { stdio: "inherit" });
