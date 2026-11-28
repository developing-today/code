// The home-resolution contract for the acpx wrapper unit (#8220 unit 3,
// #14757; contract design #5762), shaped after bin/home-dir.mjs.
//
// One way, everywhere: home is a non-empty HOME. An empty HOME counts as
// unset, and unset is a named error at the first surface that needs a path,
// before any read, write, spawn or connect derived from home. No root
// anchoring, no CWD fallback, no literal `~`, no hardcoded user and no
// passwd lookup (os.homedir() falls back to passwd when HOME is unset, which
// is exactly the substitute directory the contract forbids).
//
// This module is a sibling of muse-agent-acp-wrapper.mjs on purpose: the
// wrapper is deployed as a single file into ~/.openclaw/acpx by
// home/openclaw/default.nix, which installs this module beside it under the
// same name, so the relative import resolves in a checkout and deployed.

import path from 'node:path';

export class HomeUnsetError extends Error {
  constructor(purpose) {
    super(
      `HOME is not set or is empty: ${purpose} requires a non-empty HOME ` +
        '(home-resolution contract, issues #5762/#8220)'
    );
    this.name = 'HomeUnsetError';
  }
}

/** The non-empty HOME, or a HomeUnsetError naming `purpose`. */
export const resolveHome = (env = process.env, purpose = 'this tool') => {
  const home = env.HOME;
  if (typeof home !== 'string' || home === '') throw new HomeUnsetError(purpose);
  return home;
};

/** `parts` joined under HOME. */
export const homePath = (env, purpose, ...parts) => path.join(resolveHome(env, purpose), ...parts);
