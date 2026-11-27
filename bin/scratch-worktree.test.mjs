// Tests for per-run scratch worktrees (#11348).
// Run: node --test bin/scratch-worktree.test.mjs
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { add, remove, runId, parseArgs, validatePrefix, blockingChanges, ScratchError } from './scratch-worktree.mjs';

const script = path.join(path.dirname(fileURLToPath(import.meta.url)), 'scratch-worktree.mjs');

const sh = (args, cwd) => {
  const result = spawnSync('git', args, { cwd, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim();
};

// A bare "origin" with one commit on main, and a clone of it.
const fixture = () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'scratch-worktree-'));
  const origin = path.join(dir, 'origin.git');
  const seed = path.join(dir, 'seed');
  const repo = path.join(dir, 'repo');
  sh(['init', '-q', '--bare', '-b', 'main', origin], dir);
  sh(['init', '-q', '-b', 'main', seed], dir);
  fs.writeFileSync(path.join(seed, 'README'), 'seed\n');
  sh(['add', 'README'], seed);
  sh(['-c', 'user.name=t', '-c', 'user.email=t@t', 'commit', '-q', '-m', 'seed'], seed);
  sh(['push', '-q', origin, 'main'], seed);
  sh(['clone', '-q', origin, repo], dir);
  return { dir, seed, repo, root: path.join(dir, 'fleet') };
};

const refused = (code, fragment) => (err) =>
  err instanceof ScratchError && err.code === code && err.message.includes(fragment);

test('run ids carry a UTC timestamp and a random suffix', () => {
  const id = runId(new Date('2026-11-17T18:32:08.123Z'), Buffer.from('a1b2c3d4', 'hex'));
  assert.equal(id, '20261117T183208Z-a1b2c3d4');
  assert.notEqual(runId(), runId());
});

test('prefixes are a single plain path segment', () => {
  assert.equal(validatePrefix('improvement-issue-creator-code-lane-b'), 'improvement-issue-creator-code-lane-b');
  for (const bad of ['', '../x', 'a/b', '-x', 'a b', undefined]) {
    assert.throws(() => validatePrefix(bad), refused(2, '--prefix'));
  }
});

test('two runs of the same task in the same second never share a path', () => {
  const { repo, root } = fixture();
  const now = new Date('2026-11-17T18:32:08Z');
  const first = add({ repo, prefix: 'creator', root, now });
  const second = add({ repo, prefix: 'creator', root, now });
  assert.notEqual(first.path, second.path);
  assert.notEqual(first.token, second.token);
  assert.equal(first.head, sh(['rev-parse', 'origin/main'], repo));
});

test('an existing path is refused, never adopted', () => {
  const { repo, root } = fixture();
  const now = new Date('2026-11-17T18:32:08Z');
  const random = Buffer.from('00000000', 'hex');
  const crashed = add({ repo, prefix: 'creator', root, now, random });
  assert.throws(() => add({ repo, prefix: 'creator', root, now, random }), refused(3, `refusing to adopt existing scratch path ${crashed.path}`));
});

test('add starts from the freshly fetched base', () => {
  const { dir, seed, repo, root } = fixture();
  fs.writeFileSync(path.join(seed, 'README'), 'newer\n');
  sh(['-c', 'user.name=t', '-c', 'user.email=t@t', 'commit', '-q', '-am', 'newer'], seed);
  sh(['push', '-q', path.join(dir, 'origin.git'), 'main'], seed);
  const created = add({ repo, prefix: 'creator', root });
  assert.equal(created.head, sh(['rev-parse', 'HEAD'], seed));
});

test('a base without a remote prefix is refused rather than silently stale (#12172)', () => {
  const { dir, seed, repo, root } = fixture();
  const local = sh(['rev-parse', 'main'], repo);
  fs.writeFileSync(path.join(seed, 'README'), 'newer\n');
  sh(['-c', 'user.name=t', '-c', 'user.email=t@t', 'commit', '-q', '-am', 'newer'], seed);
  sh(['push', '-q', path.join(dir, 'origin.git'), 'main'], seed);
  assert.throws(() => add({ repo, prefix: 'creator', root, base: 'main' }), refused(2, 'pass origin/main'));
  assert.ok(!fs.existsSync(root) || fs.readdirSync(root).length === 0);
  // Opting out of the fetch keeps a local base usable, at the local ref.
  assert.equal(add({ repo, prefix: 'creator', root, base: 'main', fetch: false }).head, local);
});

test('a worktree add failure is reported, not masked', () => {
  const { repo, root } = fixture();
  assert.throws(() => add({ repo, prefix: 'creator', root, base: 'origin/no-such-branch' }), refused(4, 'git fetch'));
});

test('remove needs the owning token and a clean tree, and never forces', () => {
  const { repo, root } = fixture();
  const mine = add({ repo, prefix: 'creator', root });
  const theirs = add({ repo, prefix: 'creator', root });

  assert.throws(() => remove({ path: theirs.path, token: mine.token }), refused(3, 'belongs to another run'));

  fs.writeFileSync(path.join(mine.path, 'notes.txt'), 'unsaved work\n');
  assert.throws(() => remove({ path: mine.path, token: mine.token }), refused(3, 'uncommitted changes'));
  assert.ok(fs.existsSync(path.join(mine.path, 'notes.txt')));

  fs.rmSync(path.join(mine.path, 'notes.txt'));
  assert.deepEqual(remove({ path: mine.path, token: mine.token }), { removed: mine.path, head: mine.head });
  assert.ok(!fs.existsSync(mine.path));
  assert.ok(fs.existsSync(theirs.path));
});

test('blockingChanges excuses only the exact untracked fleet marker (#14309)', () => {
  assert.equal(blockingChanges('?? .fleet-session-id'), '');
  assert.equal(blockingChanges(''), '');
  // Any other state still blocks: another untracked file alongside the
  // marker, a modified tracked marker, or a marker below the root.
  assert.equal(blockingChanges('?? .fleet-session-id\n?? notes.txt'), '?? notes.txt');
  assert.equal(blockingChanges(' M .fleet-session-id'), ' M .fleet-session-id');
  assert.equal(blockingChanges('?? sub/.fleet-session-id'), '?? sub/.fleet-session-id');
});

test('remove succeeds when the only untracked file is the fleet session marker (#14309)', () => {
  const { repo, root } = fixture();
  const created = add({ repo, prefix: 'creator', root });
  // The fixture repo does not ignore the marker, matching openchamber,
  // opencode and darwin-nix in #14309.
  fs.writeFileSync(path.join(created.path, '.fleet-session-id'), 'ses_test\n');
  assert.deepEqual(remove({ path: created.path, token: created.token }), { removed: created.path, head: created.head });
  assert.ok(!fs.existsSync(created.path));
});

test('a fleet marker does not excuse any other untracked file (#14309)', () => {
  const { repo, root } = fixture();
  const created = add({ repo, prefix: 'creator', root });
  fs.writeFileSync(path.join(created.path, '.fleet-session-id'), 'ses_test\n');
  fs.writeFileSync(path.join(created.path, 'notes.txt'), 'unsaved work\n');
  assert.throws(() => remove({ path: created.path, token: created.token }), refused(3, 'uncommitted changes'));
  assert.ok(fs.existsSync(path.join(created.path, 'notes.txt')));
});

test('a worktree this helper did not create is never removed', () => {
  const { repo, root } = fixture();
  const foreign = path.join(root, 'creator-20261117-lane-b-3');
  fs.mkdirSync(root, { recursive: true });
  sh(['worktree', 'add', '--detach', '-q', foreign, 'origin/main'], repo);
  assert.throws(() => remove({ path: foreign, token: 'anything' }), refused(3, 'no scratch owner record'));
  assert.ok(fs.existsSync(foreign));
});

test('a missing --repo is a usage error naming the path, not a git ENOENT', () => {
  const { dir, root } = fixture();
  const missing = path.join(dir, 'no-such-checkout');
  assert.throws(() => add({ repo: missing, prefix: 'creator', root }), refused(2, `--repo ${missing} is not a directory`));
  assert.equal(fs.existsSync(root), false);
});

test('a quoted ~/ repo, root and path expand under HOME', () => {
  const { dir, repo } = fixture();
  const saved = process.env.HOME;
  process.env.HOME = dir;
  try {
    const made = add({ repo: `~/${path.basename(repo)}`, prefix: 'creator', root: '~/fleet' });
    assert.ok(made.path.startsWith(path.join(dir, 'fleet') + path.sep));
    const rel = path.relative(dir, made.path);
    assert.equal(remove({ path: `~/${rel}`, token: made.token }).removed, made.path);
  } finally {
    process.env.HOME = saved;
  }
});

test('the owner record does not dirty the tree', () => {
  const { repo, root } = fixture();
  const created = add({ repo, prefix: 'creator', root });
  assert.equal(sh(['status', '--porcelain', '--untracked-files=all'], created.path), '');
});

test('argument parsing rejects unknown commands and options', () => {
  assert.deepEqual(parseArgs(['add', '--repo', 'r', '--prefix', 'p', '--no-fetch']), {
    command: 'add',
    options: { repo: 'r', prefix: 'p', fetch: false }
  });
  assert.throws(() => parseArgs(['adopt']), refused(2, 'usage'));
  assert.throws(() => parseArgs(['add', '--force']), refused(2, '--force'));
  assert.throws(() => parseArgs(['remove', '--token']), refused(2, '--token'));
});

test('the CLI prints JSON and exits with the refusal code', () => {
  const { repo, root } = fixture();
  const run = (args) => spawnSync(process.execPath, [script, ...args], { encoding: 'utf8' });
  const added = run(['add', '--repo', repo, '--prefix', 'creator', '--root', root]);
  assert.equal(added.status, 0, added.stderr);
  const { path: created, token } = JSON.parse(added.stdout);
  const wrong = run(['remove', '--path', created, '--token', 'nope']);
  assert.equal(wrong.status, 3);
  assert.match(wrong.stderr, /belongs to another run/);
  const removed = run(['remove', '--path', created, '--token', token]);
  assert.equal(removed.status, 0, removed.stderr);
  assert.equal(JSON.parse(removed.stdout).removed, created);
});
