// Validates config.json, generates the Wrangler config, retires old Workers, deploys, and sets CALLERS.
// Usage: deploy.mjs [--dry-run]. Needs CLOUDFLARE_ACCOUNT_ID and BWS_ACCESS_TOKEN; deploy.sh sets them.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const generatedDir = join(here, ".generated");
const generatedConfig = join(generatedDir, "wrangler.json");
const KNOWN_TRANSPORTS = ["email"];
const MODES = ["template", "free"];
const PLACEHOLDER = "{code}";

const config = JSON.parse(readFileSync(join(here, "config.json"), "utf8"));
const dryRun = process.argv.includes("--dry-run");

const problems = validate(config);
if (problems.length > 0) {
  throw new Error(`config.json is invalid:\n  ${problems.join("\n  ")}`);
}

const accountId = requireEnv("CLOUDFLARE_ACCOUNT_ID");
const secrets = JSON.parse(execFileSync("bws", ["secret", "list", "-o", "json"], { encoding: "utf8" }));
const secretValue = (key) => {
  const found = secrets.find((secret) => secret.key === key);
  if (!found?.value) {
    throw new Error(`no Bitwarden secret named ${key}`);
  }
  return found.value;
};
const apiToken = secretValue("cloudflare.api-token-account-wide");
const env = { ...process.env, CLOUDFLARE_API_TOKEN: apiToken, CLOUDFLARE_ACCOUNT_ID: accountId };

const mailConfig = {
  defaultTransport: config.defaultTransport,
  transports: config.transports,
  limits: config.limits,
  templates: config.templates,
};
const callers = Object.fromEntries(
  Object.entries(config.callers).map(([name, caller]) => [
    name,
    {
      sha256: createHash("sha256").update(secretValue(caller.tokenKey)).digest("hex"),
      transports: caller.transports,
    },
  ]),
);

const wranglerConfig = {
  name: config.worker,
  main: "../src/index.js",
  compatibility_date: config.compatibilityDate,
  workers_dev: config.workersDev,
  routes: config.routes.map((pattern) => ({ pattern, custom_domain: true })),
  vars: { MAIL_CONFIG: JSON.stringify(mailConfig) },
  send_email: [{ name: "EMAIL", allowed_sender_addresses: config.senders }],
  durable_objects: { bindings: [{ name: "LIMITER", class_name: "Limiter" }] },
  migrations: [{ tag: "v1", new_sqlite_classes: ["Limiter"] }],
};
mkdirSync(generatedDir, { recursive: true });
writeFileSync(generatedConfig, `${JSON.stringify(wranglerConfig, null, 2)}\n`);

wrangler(["deploy", "--dry-run"]);
if (dryRun) {
  console.log(`dry run: ${config.worker} config is valid, nothing deployed`);
  process.exit(0);
}

// A hostname can belong to only one Worker, so retired Workers go before the deploy takes their routes.
for (const name of config.retire) {
  await retire(name);
}
wrangler(["deploy"]);
wrangler(["secret", "put", "CALLERS"], JSON.stringify(callers));
console.log(`deployed ${config.worker} with callers: ${Object.keys(callers).join(", ")}`);

async function retire(name) {
  const response = await fetch(`https://api.cloudflare.com/client/v4/accounts/${accountId}/workers/scripts/${name}`, {
    headers: { authorization: `Bearer ${apiToken}` },
  });
  if (response.status === 404) {
    console.log(`retire: ${name} is already gone`);
    return;
  }
  if (!response.ok) {
    throw new Error(`retire: checking ${name} failed with HTTP ${response.status}`);
  }
  wrangler(["delete", "--name", name, "--force"]);
}

function wrangler(args, input) {
  execFileSync("wrangler", [...args, "--config", generatedConfig], {
    env,
    input,
    stdio: [input === undefined ? "inherit" : "pipe", "inherit", "inherit"],
  });
}

function requireEnv(name) {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is not set; run deploy.sh`);
  }
  return value;
}

function validate(c) {
  const errors = [];
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
  check(
    (c.transports ?? []).every((name) => KNOWN_TRANSPORTS.includes(name)),
    `transports must be among ${KNOWN_TRANSPORTS.join(", ")}`,
  );
  check((c.transports ?? []).includes(c.defaultTransport), "defaultTransport must be one of transports");
  check(isPositiveInt(c.limits?.windowSeconds), "limits.windowSeconds must be a positive integer");
  check(isPositiveInt(c.limits?.callerPerWindow), "limits.callerPerWindow must be a positive integer");
  check(isPositiveInt(c.limits?.recipientPerWindow), "limits.recipientPerWindow must be a positive integer");
  check(Array.isArray(c.templates), "templates must be a list");
  for (const template of c.templates ?? []) {
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
    check(typeof caller.tokenKey === "string", `callers.${name}.tokenKey must be a Bitwarden secret name`);
    const grants = Object.entries(caller.transports ?? {});
    check(grants.length > 0, `callers.${name}.transports must grant at least one transport`);
    for (const [transport, grant] of grants) {
      const where = `callers.${name}.transports.${transport}`;
      check(c.transports?.includes(transport), `${where} is not an enabled transport`);
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
