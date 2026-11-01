// Tests for the per-proxy CLI workroot isolation (#2785).
// Run: node --test bin/cli-proxy-workroots.test.mjs
//
// home/common/default.nix points all three CLI proxies at one shared
// WORKDIR before this fix: commandcode and claude-code derive identical
// sessions/<sha256(key)[:16]> workdirs for overlapping keys, and the
// muse proxy uses the shared root itself as its session cwd and
// workspaceRoot. The units must give each proxy its own subtree while
// the commandcode/claude-code session map stays intentionally shared
// (#3067) — it is anchored to the state root, not to either workroot.
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const binDir = path.dirname(fileURLToPath(import.meta.url));
const nix = fs.readFileSync(path.join(binDir, '..', 'home', 'common', 'default.nix'), 'utf8');

const envValue = (name) => {
  const m = nix.match(new RegExp(`"${name}=([^"]+)"`));
  assert.ok(m, `default.nix should set ${name}`);
  return m[1];
};

const WORKDIRS = {
  commandcode: envValue('COMMANDCODE_PROXY_WORKDIR'),
  claudeCode: envValue('CLAUDE_CODE_PROXY_WORKDIR'),
  muse: envValue('MUSE_PROXY_WORKDIR'),
};

test('each CLI proxy unit gets a distinct workroot subtree', () => {
  const values = Object.values(WORKDIRS);
  assert.equal(new Set(values).size, 3, `workroots must be pairwise distinct: ${values}`);
  for (const [proxy, dir] of Object.entries(WORKDIRS)) {
    assert.ok(
      dir.startsWith('%h/.local/state/cli-proxies/'),
      `${proxy} workroot stays under the cli-proxies state root: ${dir}`,
    );
    assert.notEqual(
      dir,
      '%h/.local/state/cli-proxies',
      `${proxy} workroot must not be the shared root itself`,
    );
  }
});

test('an overlapping conversation key derives disjoint session workdirs', () => {
  // Both proxies derive sessions/<first 16 hex of sha256(key)> under
  // their WORKROOT (commandcode-proxy.mjs / claude-code-proxy.mjs
  // sessionWorkdirFor). Distinct workroots make the derived paths
  // disjoint even when the key — and therefore the hash — is identical.
  const key = 'an overlapping conversation key';
  const hash = crypto.createHash('sha256').update(key).digest('hex').slice(0, 16);
  const ccDir = path.join(WORKDIRS.commandcode, 'sessions', hash);
  const claudeDir = path.join(WORKDIRS.claudeCode, 'sessions', hash);
  assert.notEqual(ccDir, claudeDir);
});

test('the shared commandcode/claude session map is unchanged (#3067)', () => {
  const cc = envValue('COMMANDCODE_PROXY_SESSION_FILE');
  const claude = envValue('CLAUDE_CODE_PROXY_SESSION_FILE');
  assert.equal(cc, claude, 'both units must name the same session file');
  assert.equal(cc, '%h/.local/state/cli-proxies/sessions.json');
  // The shared map lives at the state root, outside every workroot.
  for (const dir of Object.values(WORKDIRS)) {
    assert.ok(!cc.startsWith(dir), 'session file must not sit inside a per-proxy workroot');
  }
});
