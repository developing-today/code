#!/usr/bin/env node
// Per-run scratch worktrees for scheduled fleet tasks (#11348).
//
// The improvement-issue-creator briefs built their scratch path from the
// date and a per-day counter, guarded it with `test -d … || worktree add`,
// and cleaned up with `git worktree remove --force`. A crashed run left its
// tree behind; the next run computing the same counter adopted it (skipping
// fetch and base checks) and then force-removed a tree it never created.
//
// This helper replaces that shape:
//
//   add     names the path with a per-run unique id (UTC timestamp plus
//           random suffix), refuses loudly if the path already exists,
//           fetches the base (which must name a remote, such as
//           origin/main, unless --no-fetch is given), runs `git worktree add` without masking its
//           exit status, and records an owner token in the worktree's own
//           git dir (so the tree stays clean).
//   remove  removes a scratch worktree only when the caller presents the
//           token `add` printed and the tree is clean; never --force. It
//           prints what it removed. A tree whose only untracked file is
//           the fleet-owned `.fleet-session-id` marker counts as clean
//           (#14309): the fleet writes that marker into session
//           worktrees, and repos that do not ignore it would otherwise
//           make every scratch tree unremovable by this tool.
//
// Usage:
//   node bin/scratch-worktree.mjs add --repo <checkout> --prefix <name>
//        [--root <dir>] [--base origin/main] [--no-fetch]
//     prints JSON {"path": …, "token": …, "base": …, "head": …}
//   node bin/scratch-worktree.mjs remove --path <worktree> --token <token>
//
// Exit codes: 0 ok, 2 usage, 3 refused (path exists, wrong owner, dirty),
// 4 git failed.

import { spawnSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { expandTilde, homePath } from './home-dir.mjs';

export const OWNER_FILE = 'scratch-worktree-owner';

export class ScratchError extends Error {
  constructor(code, message) {
    super(message);
    this.code = code;
  }
}

const git = (args, cwd) => {
  const result = spawnSync('git', args, { cwd, encoding: 'utf8' });
  if (result.error) throw new ScratchError(4, `git ${args.join(' ')}: ${result.error.message}`);
  if (result.status !== 0) {
    throw new ScratchError(
      4,
      `git ${args.join(' ')} exited ${result.status}: ${(result.stderr || result.stdout).trim()}`
    );
  }
  return result.stdout.trim();
};

// A prefix names the task, not the run: plain path-segment characters only.
export const validatePrefix = (prefix) => {
  if (typeof prefix !== 'string' || !/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(prefix)) {
    throw new ScratchError(2, `--prefix "${prefix}" must be letters, digits, '.', '_' or '-'`);
  }
  return prefix;
};

export const runId = (now = new Date(), random = randomBytes(4)) => {
  const stamp = now.toISOString().replace(/[-:]/g, '').replace(/\.\d+Z$/, 'Z');
  return `${stamp}-${random.toString('hex')}`;
};

export const scratchPath = (root, prefix, id) => path.join(root, `${validatePrefix(prefix)}-${id}`);

const ownerFile = (worktree) => {
  const gitDir = git(['rev-parse', '--absolute-git-dir'], worktree);
  return path.join(gitDir, OWNER_FILE);
};

export const add = ({ repo, prefix, root, base = 'origin/main', fetch = true, now, random }) => {
  if (!repo) throw new ScratchError(2, '--repo is required');
  validatePrefix(prefix);
  // A quoted `~/…` reaches us unexpanded, and a missing checkout otherwise
  // surfaces as Node's "spawnSync git ENOENT", which reads like git is gone.
  repo = expandTilde(repo, process.env, 'scratch-worktree --repo');
  if (!fs.existsSync(repo) || !fs.statSync(repo).isDirectory()) {
    throw new ScratchError(2, `--repo ${repo} is not a directory`);
  }
  const scratchRoot =
    root !== undefined
      ? expandTilde(root, process.env, 'scratch-worktree --root')
      : homePath(process.env, 'scratch-worktree (default --root)', 'git', 'fleet');
  const target = scratchPath(scratchRoot, prefix, runId(now, random));
  if (fs.existsSync(target)) {
    throw new ScratchError(3, `refusing to adopt existing scratch path ${target}`);
  }
  if (fetch) {
    // Only a remote-qualified base can be fetched. A bare branch name such
    // as `main` would otherwise skip the fetch and branch from a stale local
    // ref while the output still looks fresh (#12172), so it is refused
    // unless the caller opts out of fetching with --no-fetch.
    const slash = base.indexOf('/');
    if (slash <= 0) {
      throw new ScratchError(
        2,
        `--base ${base} has no remote prefix, so it cannot be fetched; pass origin/${base.replace(/^\/+/, '')} (or --no-fetch to use the local ref)`,
      );
    }
    git(['fetch', '-q', base.slice(0, slash), base.slice(slash + 1)], repo);
  }
  const baseSha = git(['rev-parse', '--verify', `${base}^{commit}`], repo);
  fs.mkdirSync(scratchRoot, { recursive: true });
  git(['worktree', 'add', '--detach', '-q', target, baseSha], repo);
  const head = git(['rev-parse', 'HEAD'], target);
  if (head !== baseSha) {
    throw new ScratchError(4, `worktree ${target} is at ${head}, expected ${base} ${baseSha}`);
  }
  const token = randomBytes(16).toString('hex');
  fs.writeFileSync(ownerFile(target), `${token}\n`, { mode: 0o600 });
  return { path: target, token, base, head };
};

// The fleet session marker: written by the fleet into the worktree a
// session runs in, never part of the tree's own work. Only the exact
// untracked root entry is excused — a tracked or modified marker, a
// marker in a subdirectory, or any other untracked file still blocks
// removal (#14309).
export const FLEET_SESSION_MARKER = '.fleet-session-id';

export const blockingChanges = (status) =>
  status
    .split('\n')
    .filter((line) => line !== '' && line !== `?? ${FLEET_SESSION_MARKER}`)
    .join('\n');

export const remove = ({ path: target, token }) => {
  if (!target) throw new ScratchError(2, '--path is required');
  if (!token) throw new ScratchError(2, '--token is required');
  target = expandTilde(target, process.env, 'scratch-worktree --path');
  if (!fs.existsSync(target)) throw new ScratchError(3, `no worktree at ${target}`);
  let recorded;
  try {
    recorded = fs.readFileSync(ownerFile(target), 'utf8').trim();
  } catch {
    throw new ScratchError(3, `refusing to remove ${target}: it has no scratch owner record, so this run did not create it`);
  }
  if (recorded !== token) {
    throw new ScratchError(3, `refusing to remove ${target}: it belongs to another run`);
  }
  const status = blockingChanges(git(['status', '--porcelain', '--untracked-files=all'], target));
  if (status !== '') {
    throw new ScratchError(3, `refusing to remove ${target}: it has uncommitted changes:\n${status}`);
  }
  const head = git(['rev-parse', 'HEAD'], target);
  git(['worktree', 'remove', target], target);
  return { removed: target, head };
};

export const parseArgs = (argv) => {
  const [command, ...rest] = argv;
  const options = {};
  for (let i = 0; i < rest.length; i += 1) {
    const flag = rest[i];
    if (flag === '--no-fetch') {
      options.fetch = false;
      continue;
    }
    const match = /^--(repo|prefix|root|base|path|token)$/.exec(flag);
    if (!match || i + 1 >= rest.length) throw new ScratchError(2, `unknown or incomplete option ${flag}`);
    options[match[1]] = rest[i + 1];
    i += 1;
  }
  if (command !== 'add' && command !== 'remove') {
    throw new ScratchError(2, 'usage: scratch-worktree.mjs add|remove [options]');
  }
  return { command, options };
};

export const main = (argv) => {
  try {
    const { command, options } = parseArgs(argv);
    const result = command === 'add' ? add(options) : remove(options);
    process.stdout.write(`${JSON.stringify(result)}\n`);
    return 0;
  } catch (err) {
    process.stderr.write(`scratch-worktree: ${err.message}\n`);
    return err instanceof ScratchError ? err.code : 4;
  }
};

if (process.argv[1] && fileURLToPath(import.meta.url) === fs.realpathSync(process.argv[1])) {
  process.exitCode = main(process.argv.slice(2));
}
