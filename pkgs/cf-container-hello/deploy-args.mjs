// Strict CLI parser so typos cannot silently trigger a live Worker deploy.
export const USAGE = [
  "usage: deploy.mjs publish-image [--dry-run]",
  "       deploy.mjs activate --version <id> [--dry-run]",
].join("\n");

export function parseArgs(argv) {
  const [command, ...args] = argv;
  if (!["publish-image", "activate"].includes(command)) {
    throw new Error(`expected publish-image or activate\n${USAGE}`);
  }
  let dryRun = false;
  let version = null;
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i];
    if (arg === "--dry-run" && !dryRun) {
      dryRun = true;
      continue;
    }
    if (arg === "--version" && command === "activate" && version === null) {
      const value = args[i + 1];
      if (!value || value.startsWith("-")) throw new Error(`--version needs an id\n${USAGE}`);
      if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value)) {
        throw new Error(`--version must be a UUID\n${USAGE}`);
      }
      version = value;
      i += 1;
      continue;
    }
    throw new Error(`unrecognized argument ${JSON.stringify(arg)}\n${USAGE}`);
  }
  return { command, dryRun, version };
}
