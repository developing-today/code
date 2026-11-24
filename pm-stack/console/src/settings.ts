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
