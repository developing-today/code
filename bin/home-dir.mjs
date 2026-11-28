// The home-resolution contract for the bin/ family (#8220 unit 2, #11205;
// contract design #5762).
//
// One way, everywhere: home is a non-empty HOME. An empty HOME counts as
// unset, and unset is a named error at the first surface that needs a path,
// before any read, write, spawn or connect derived from home. No root
// anchoring, no CWD fallback, no literal `~`, no hardcoded user and no
// passwd lookup (os.homedir() falls back to passwd when HOME is unset, which
// is exactly the substitute directory the contract forbids). An absolute
// XDG override keeps working; a relative or empty one is ignored, as the XDG
// base-directory spec requires, and resolution falls back to HOME.
//
// Every bin/ script resolves home through this module. These scripts are
// launched from a checkout or as `node ${../../bin}/<name>.mjs` from the
// store, so the sibling import resolves in both places.

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

/** A `~` or `~/…` path expanded under HOME; any other path is returned as is. */
export const expandTilde = (value, env = process.env, purpose = 'this tool') => {
  if (value === '~') return resolveHome(env, purpose);
  if (typeof value === 'string' && value.startsWith('~/')) return homePath(env, purpose, value.slice(2));
  return value;
};

/** An XDG base directory: the variable when it is absolute, else HOME/<fallback>. */
export const xdgDir = (env, name, fallback, purpose = 'this tool') => {
  const value = env[name];
  if (typeof value === 'string' && value !== '' && path.isAbsolute(value)) return value;
  return homePath(env, purpose, ...fallback);
};

export const xdgStateHome = (env = process.env, purpose = 'this tool') =>
  xdgDir(env, 'XDG_STATE_HOME', ['.local', 'state'], purpose);
