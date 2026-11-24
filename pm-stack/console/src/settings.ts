// The platform's own settings: the ones the repository decides.
//
// They are written here once, and both sides carry them: the console on
// a machine (as what it uses when its environment says nothing else),
// and the Worker (bundled into every deploy from main). A machine and
// Cloudflare that both follow main therefore agree, and one that does
// not is found out by comparing what it says with this.
//
// Not here: who may sign in (the owners and the sign-in issuers are set
// when the hostname is, and are the deployment's), and nothing about a
// single machine (its name, its files, its units).

export interface PlatformSettings {
  /** The repository the work is in. */
  repo: string;
  /** The queue the board shows. */
  queue: string;
  /** Where the queue's store answers. */
  queueDbUrl: string;
  /** What the queue store's token is called where secrets are kept: a
   * name, never the token. */
  queueTokenName: string;
  /** The Android app's package. */
  androidPackage: string;
  /** Label on an issue that is an instruction from the owner. */
  promptLabel: string;
  /** Repositories looked in for what is labelled as needing the owner,
   * and whose items may be answered from "Needs me". The ones the
   * dashboard's census covers (REPOS in pm-stack/aggregator/src/model.ts;
   * a test holds the two lists to each other). By their present names:
   * two forks moved to another organisation and a third was renamed.
   * GitHub still answers the old names with the present repositories,
   * but an item named by an old name is not the item the dashboard
   * names, and a reply is taken only for an item in a repository on
   * this list. */
  needsRepos: string[];
}

/** The names, in the order they are said in. */
export const PLATFORM_SETTING_NAMES = [
  "repo",
  "queue",
  "queueDbUrl",
  "queueTokenName",
  "androidPackage",
  "promptLabel",
  "needsRepos",
] as const satisfies readonly (keyof PlatformSettings)[];

/** The settings as the repository has them. A new object each time:
 * whoever is handed one may change it without changing anyone else's. */
export function platformSettings(): PlatformSettings {
  return {
    repo: "developing-today/code",
    queue: "repo-health-code-pilot",
    queueDbUrl: "https://fleet-queue-dezren39.aws-us-east-1.turso.io",
    queueTokenName: "turso.fleet-queue-rw-token",
    androidPackage: "today.developing.pmconsole",
    promptLabel: "owner/prompt",
    needsRepos: [
      "developing-today/code",
      "developing-today-forks/opencode",
      "developing-today-forks/openchamber",
      "dezren39/darwin-nix",
    ],
  };
}

/** The settings the platform goes by its own for, whatever a machine
 * says: the Worker's are the ones its deploy carries, which is main's
 * word. Where the queue's store answers is among them, and that is
 * where the queue's token is sent: no machine's report moves it. */
export const PLATFORM_OWN = [
  "repo",
  "queue",
  "queueDbUrl",
  "queueTokenName",
  "androidPackage",
  "promptLabel",
] as const satisfies readonly (keyof PlatformSettings)[];

/** The one the platform still goes by a reporting machine's for. A
 * machine takes a reply, or "done", only for an item in a repository
 * on its own list, and what the platform cannot do itself it leaves to
 * a machine: reading by another list than the machine's would show
 * items the machine then refuses. It becomes the platform's own with
 * what makes that safe: a console that takes the platform's list, or
 * an action sent only to a machine that lists the repository (#14163,
 * #13492). */
export const MACHINES_FOR_NOW = ["needsRepos"] as const satisfies readonly (keyof PlatformSettings)[];

/** What the platform goes by, given what a machine told: its own for
 * what is its own, and the machine's for the rest (who may sign in,
 * which is the deployment's, and the list above). A new object. */
export function goesBy<T extends PlatformSettings>(told: T, platform: PlatformSettings = platformSettings()): T {
  const own = Object.fromEntries(PLATFORM_OWN.map((name) => [name, platform[name]]));
  return { ...told, ...own };
}

/** One setting a machine runs with another value of: what it says,
 * what the platform has, and which of the two the platform goes by. */
export interface SettingDiffers {
  name: keyof PlatformSettings;
  machine: unknown;
  platform: unknown;
  goesBy: "platform" | "machine";
}

/** How much of what a machine says is shown: it is the machine's word,
 * and is not shown without end. */
const SHOWN_CHARS = 200;
const SHOWN_ITEMS = 50;
function shown(value: unknown): unknown {
  if (Array.isArray(value)) return value.slice(0, SHOWN_ITEMS).map((item) => String(item).slice(0, SHOWN_CHARS));
  return typeof value === "string" ? value.slice(0, SHOWN_CHARS) : value === undefined ? null : value;
}

/** The settings a machine runs with others of, each with both values,
 * for showing to the owner. */
export function settingsDifferences(
  told: Partial<Record<keyof PlatformSettings, unknown>>,
  platform: PlatformSettings = platformSettings(),
): SettingDiffers[] {
  const machines = new Set<string>(MACHINES_FOR_NOW);
  return settingsThatDiffer(told, platform).map((name) => ({
    name,
    machine: shown(told[name]),
    platform: platform[name],
    goesBy: machines.has(name) ? "machine" : "platform",
  }));
}

function same(a: unknown, b: unknown): boolean {
  if (Array.isArray(a) || Array.isArray(b)) {
    return Array.isArray(a) && Array.isArray(b) && a.length === b.length && a.every((item, i) => item === b[i]);
  }
  return a === b;
}

/** The names of the settings in which `told` is not what the platform
 * has, in the order they are said in. A list in another order is
 * another list: what is shown follows its order. A setting `told` does
 * not carry at all differs like any other. */
export function settingsThatDiffer(
  told: Partial<Record<keyof PlatformSettings, unknown>>,
  platform: PlatformSettings = platformSettings(),
): (keyof PlatformSettings)[] {
  return PLATFORM_SETTING_NAMES.filter((name) => !same(told[name], platform[name]));
}
