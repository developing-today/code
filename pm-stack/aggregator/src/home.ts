// Home resolution for the aggregator unit, per the #5762 home-resolution
// contract (#8220). Node's os.homedir() falls back to the passwd entry when
// HOME is unset, which the contract declined as a silent substitution. Here an
// unset or empty HOME is a named error that says which setting needed it, and
// a set HOME is returned unchanged, so every derived path stays the same.

export class HomeUnresolvedError extends Error {
  /** The setting whose default needed HOME, e.g. PM_AGG_FRESHNESS_FILE. */
  readonly setting: string;

  constructor(setting: string, home: string | undefined) {
    super(
      `HOME is ${home === undefined ? "unset" : "empty"}, so the default for ${setting} cannot be derived; ` +
        `set HOME or set ${setting} (home-resolution contract #5762: no passwd fallback)`,
    );
    this.name = "HomeUnresolvedError";
    this.setting = setting;
  }
}

/** HOME for a default path, or HomeUnresolvedError when it is unset or blank. */
export function requireHome(setting: string, env: Record<string, string | undefined> = process.env): string {
  const home = env.HOME;
  if (home === undefined || home.trim() === "") throw new HomeUnresolvedError(setting, home);
  return home;
}
