// Validates config.json and uploads a Cloudflare Workers Version. Uploading does not promote it,
// change routes, or retire another Worker. Promotion and resource retirement are separate actions.
// Usage: deploy.mjs [--dry-run]. Needs CLOUDFLARE_ACCOUNT_ID; uploads also need CLOUDFLARE_API_TOKEN.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const cfBin = join(here, "node_modules", ".bin", "cf");
const KNOWN_TRANSPORTS = ["email"];
const MODES = ["template", "free"];
const PLACEHOLDER = "{code}";

// Strict argument validation (#3281): the only accepted invocation is
// `deploy.mjs [--dry-run]`. Anything else — a typo'd `--dry-rnu`, a
// stray positional, an unknown flag — aborts here, before any config,
// cf or API work: unrecognized arguments must never fall through to an upload.
const args = process.argv.slice(2);
const unknownArgs = args.filter((arg) => arg !== "--dry-run");
if (args.length > 1 || unknownArgs.length > 0) {
  // unknownArgs empty on this branch means every arg was --dry-run, so a
  // length failure is a duplicate, not an unrecognized argument (#7855).
  const message =
    unknownArgs.length > 0
      ? `unrecognized argument(s): ${unknownArgs.join(" ")}`
      : `duplicate argument(s): ${[...new Set(args)].join(" ")}`;
  console.error(`${message}\nusage: deploy.mjs [--dry-run]`);
  process.exit(2);
}
const dryRun = args.includes("--dry-run");

const config = JSON.parse(readFileSync(join(here, "config.json"), "utf8"));

const problems = validate(config);
if (problems.length > 0) {
  throw new Error(`config.json is invalid:\n  ${problems.join("\n  ")}`);
}

requireEnv("CLOUDFLARE_ACCOUNT_ID");
if (!dryRun) requireEnv("CLOUDFLARE_API_TOKEN");

// cf resolves cloudflare.config.ts from this project's directory. Install the exact
// locked project dependency on a first local invocation, matching the pkarr Worker flow.
if (!existsSync(cfBin)) {
  console.log("cf dependency missing; running npm ci");
  execFileSync("npm", ["ci", "--no-audit", "--no-fund"], { cwd: here, stdio: "inherit" });
}

const sha = process.env.GITHUB_SHA ?? execFileSync("git", ["rev-parse", "HEAD"], { cwd: here, encoding: "utf8" }).trim();
const tag = `git-${sha.slice(0, 12)}`;
const cfArgs = [
  "workers", "versions", "create",
  "--worker", config.worker,
  "--tag", tag,
  "--message", `publish ${sha} for ${config.worker} (upload only)`,
];
if (dryRun) cfArgs.push("--dry-run");
execFileSync(cfBin, cfArgs, { cwd: here, stdio: "inherit" });
console.log(dryRun
  ? `dry run: ${config.worker} version is valid; nothing uploaded`
  : `uploaded ${config.worker} version with tag ${tag}; promotion is a separate operation`);

function requireEnv(name) {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is not set`);
  }
  return value;
}

function validate(c) {
  const errors = [];
  if (c === null || typeof c !== "object" || Array.isArray(c)) {
    errors.push("config must be a JSON object");
    return errors;
  }
  const check = (ok, message) => {
    if (!ok) {
      errors.push(message);
    }
  };
  const isEmail = (value) => typeof value === "string" && /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value);
  const isStringList = (value) => Array.isArray(value) && value.every((item) => typeof item === "string");
  const isPositiveInt = (value) => Number.isInteger(value) && value > 0;

  check(/^[a-z0-9-]+$/.test(c.worker ?? ""), "worker must be a lowercase name");
  check(/^\d{4}-\d{2}-\d{2}$/.test(c.compatibilityDate ?? ""), "compatibilityDate must be YYYY-MM-DD");
  check(typeof c.workersDev === "boolean", "workersDev must be true or false");
  check(isStringList(c.routes) && c.routes.length > 0, "routes must be a non-empty list of hostnames");
  check(isStringList(c.retire ?? []) && !(c.retire ?? []).includes(c.worker), "retire must list other Workers");
  check(isStringList(c.senders) && c.senders.length > 0 && c.senders.every(isEmail), "senders must be email addresses");
  check(isStringList(c.transports) && c.transports.length > 0, "transports must be a non-empty list");
  const transportList = Array.isArray(c.transports) ? c.transports : [];
  check(
    transportList.every((name) => KNOWN_TRANSPORTS.includes(name)),
    `transports must be among ${KNOWN_TRANSPORTS.join(", ")}`,
  );
  check(transportList.includes(c.defaultTransport), "defaultTransport must be one of transports");
  check(isPositiveInt(c.limits?.windowSeconds), "limits.windowSeconds must be a positive integer");
  check(isPositiveInt(c.limits?.callerPerWindow), "limits.callerPerWindow must be a positive integer");
  check(isPositiveInt(c.limits?.recipientPerWindow), "limits.recipientPerWindow must be a positive integer");
  check(Array.isArray(c.templates), "templates must be a list");
  for (const [index, template] of (Array.isArray(c.templates) ? c.templates : []).entries()) {
    if (template === null || typeof template !== "object") {
      check(false, `templates[${index}] must be an object with subject and body`);
      continue;
    }
    check(
      typeof template.subject === "string" && !/[\r\n]/.test(template.subject),
      "each template subject must be one line of text",
    );
    check(
      typeof template.body === "string" && template.body.includes(PLACEHOLDER),
      `each template body must contain ${PLACEHOLDER}`,
    );
  }
  check(c.callers && typeof c.callers === "object" && Object.keys(c.callers).length > 0, "callers must not be empty");
  for (const [name, caller] of Object.entries(c.callers ?? {})) {
    if (caller === null || typeof caller !== "object") {
      check(false, `callers.${name} must be an object`);
      continue;
    }
    check(typeof caller.tokenKey === "string", `callers.${name}.tokenKey must be a Bitwarden secret name`);
    const grants = Object.entries(caller.transports ?? {});
    check(grants.length > 0, `callers.${name}.transports must grant at least one transport`);
    for (const [transport, grant] of grants) {
      const where = `callers.${name}.transports.${transport}`;
      if (grant === null || typeof grant !== "object") {
        check(false, `${where} must be an object`);
        continue;
      }
      check(transportList.includes(transport), `${where} is not an enabled transport`);
      check(MODES.includes(grant.mode), `${where}.mode must be ${MODES.join(" or ")}`);
      check(
        isStringList(grant.from) && grant.from.length > 0 && grant.from.every((from) => c.senders?.includes(from)),
        `${where}.from must be non-empty and within senders`,
      );
      check(isPositiveInt(grant.maxRecipients), `${where}.maxRecipients must be a positive integer`);
      check(grant.mode !== "template" || (c.templates ?? []).length > 0, `${where} needs templates in template mode`);
    }
  }
  return errors;
}
