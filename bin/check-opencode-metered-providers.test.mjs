import { test } from 'node:test';
import assert from 'node:assert/strict';
import { checkConfig, inspectConfig, parseJsoncTree } from './check-opencode-metered-providers.mjs';

const cleanConfig = `{
  // URLs and comment punctuation are not syntax.
  "description": "https://example.invalid/a//b, ]",
  "disabled_providers": ["cloudflare-gateway", "deepseek",],
  "model": "opencode-route/workhorse",
  "small_model": "opencode-route/speed",
  "agent": { "build": { "model": "openai/gpt-6" } },
  "experimental": {
    "provider_routes": { "openai-plan": { "providers": ["openai", "openai.alternate"] } },
    "model_routes": {
      "workhorse": { "targets": ["openai-plan/gpt-6", { "model": "openai/gpt-6", "window": "peak" }] },
      "nested": { "targets": ["opencode-route/workhorse"], "fallbacks": ["openai/gpt-5"] },
    },
  },
}`;

test('parses JSONC comments, URLs, and trailing commas without losing strings', () => {
  const { root } = parseJsoncTree(cleanConfig);
  assert.equal(root.type, 'object');
  const result = inspectConfig(cleanConfig);
  assert.deepEqual(result.issues, []);
  assert.ok(result.references.some((reference) => reference.value === 'openai/gpt-6'));
});

test('handles CR-only line endings and keeps the exception adjacent', () => {
  const config = '{\r// payg-exception: approved for a one-off test\r"model": "deepseek/deepseek-chat"\r}';
  assert.deepEqual(checkConfig(config), []);
});

test('uses the last duplicate property, matching JSON parsing behavior', () => {
  const config = `{
    "agent": { "coder": { "model": "deepseek/old", "model": "openai/gpt-6" } },
  }`;
  assert.deepEqual(checkConfig(config), []);
});

test('rejects metered providers in string/object route targets, fallbacks, and defaults, even when disabled', () => {
  const config = `{
    "disabled_providers": ["cloudflare-ai-gateway"],
    "model": "cloudflare-ai-gateway/@cf/meta/llama",
    "agent": {
      "coder": {
        "model": "cloudflare-workers-ai/@cf/qwen",
        "fallback": ["openai/gpt-6", "deepseek/deepseek-chat"],
      },
    },
    "experimental": {
      "model_routes": {
        "outer": { "targets": ["opencode-route/inner"] },
        "inner": { "targets": [{ "model": "cloudflare-workers-ai/@cf/qwen" }] },
        "reserve": { "targets": ["openai/gpt-6"], "fallbacks": ["cloudflare-ai-gateway/@cf/qwen"] },
      },
    },
  }`;
  const issues = checkConfig(config);
  assert.equal(issues.length, 5);
  assert.ok(issues.some((issue) => issue.includes('$.model')));
  assert.ok(issues.some((issue) => issue.includes('$.agent.coder.model')));
  assert.ok(issues.some((issue) => issue.includes('$.agent.coder.fallback[1]')));
  assert.ok(issues.some((issue) => issue.includes('$.experimental.model_routes.inner.targets[0].model')));
  assert.ok(issues.some((issue) => issue.includes('$.experimental.model_routes.reserve.fallbacks[0]')));
});

test('rejects a PAYG provider selected through a provider route', () => {
  const config = `{
    "experimental": { "provider_routes": { "payg": { "providers": ["deepseek"] } } },
  }`;
  const issues = checkConfig(config);
  assert.equal(issues.length, 1);
  assert.match(issues[0], /provider-route/);
  assert.match(issues[0], /deepseek/);
});

test('allows a non-empty adjacent exception for each specific metered selection', () => {
  const config = `{
    // payg-exception: explicitly approved for the one-off migration
    "model": "deepseek/deepseek-chat",
    "experimental": {
      "model_routes": {
        "review": {
          "targets": [
            // payg-exception: approved benchmark fallback
            { "model": "cloudflare-gateway/@cf/qwen" },
          ],
        },
      },
    },
  }`;
  assert.deepEqual(checkConfig(config), []);
});

test('rejects empty, misplaced, and stale exception markers', () => {
  const config = `{
    // payg-exception:
    "model": "deepseek/deepseek-chat",
    // payg-exception: stale because this selection is not metered
    "small_model": "openai/gpt-6",
    // payg-exception: not attached to a model selection
    "description": "ordinary text",
  }`;
  const issues = checkConfig(config);
  assert.equal(issues.length, 4);
  assert.ok(issues.some((issue) => issue.includes('non-empty reason')));
  assert.ok(issues.some((issue) => issue.includes('non-metered selection')));
  assert.ok(issues.some((issue) => issue.includes('exactly one model/provider selection')));
  assert.ok(issues.some((issue) => issue.includes('metered provider "deepseek"')));
});

test('rejects metered providers in top-level model_routes when no experimental map exists', () => {
  const config = `{
    "model": "opencode-route/workhorse",
    "model_routes": {
      "workhorse": { "targets": ["deepseek/deepseek-chat"] },
    },
  }`;
  const issues = checkConfig(config);
  assert.equal(issues.length, 1);
  assert.ok(issues[0].includes('$.model_routes.workhorse.targets[0]'));
  assert.match(issues[0], /deepseek/);
});

test('rejects a PAYG provider selected through a top-level provider route', () => {
  const config = `{
    "provider_routes": { "payg": { "providers": ["deepseek"] } },
  }`;
  const issues = checkConfig(config);
  assert.equal(issues.length, 1);
  assert.match(issues[0], /provider-route/);
  assert.ok(issues[0].includes('$.provider_routes.payg.providers[0]'));
});

test('experimental route maps take precedence over top-level ones, mirroring the runtime fallback', () => {
  const config = `{
    "model_routes": {
      "workhorse": { "targets": ["deepseek/deepseek-chat"] },
    },
    "experimental": {
      "model_routes": {
        "workhorse": { "targets": ["openai/gpt-6"] },
      },
    },
  }`;
  assert.deepEqual(checkConfig(config), []);
});

test('canonical OpenCode config passes the static guard', async () => {
  const { readFile } = await import('node:fs/promises');
  const { resolve } = await import('node:path');
  const config = await readFile(resolve('.opencode/opencode.jsonc'), 'utf8');
  assert.deepEqual(checkConfig(config), []);
});

test('rejects a metered target in a model route named like a selection field', () => {
  const config = `{
    "model": "opencode-route/model",
    "experimental": {
      "model_routes": { "model": { "targets": ["deepseek/deepseek-chat"] } },
    },
  }`;
  const issues = checkConfig(config);
  assert.equal(issues.length, 1);
  assert.match(issues[0], /deepseek/);
  assert.ok(issues[0].includes('$.experimental.model_routes.model.targets[0]'));
});

test('rejects a PAYG provider in a provider group named providers', () => {
  const config = `{
    "experimental": {
      "provider_routes": { "providers": { "providers": ["deepseek"] } },
    },
  }`;
  const issues = checkConfig(config);
  assert.equal(issues.length, 1);
  assert.match(issues[0], /provider-route/);
  assert.ok(issues[0].includes('$.experimental.provider_routes.providers.providers[0]'));
});

// #4694: the entry-point check must run main() when the guard is invoked
// through a symlink (file or directory). Before the realpath fix both
// invocations silently exited 0 without checking anything.
import { spawnSync } from 'node:child_process';
import { mkdtempSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const guardPath = fileURLToPath(new URL('./check-opencode-metered-providers.mjs', import.meta.url));
const meteredConfig = '{ "model": "deepseek/deepseek-chat" }';
const cleanCliConfig = '{ "model": "openai/gpt-6" }';

function runGuard(scriptPath, configSource) {
  const dir = mkdtempSync(join(tmpdir(), 'metered-guard-'));
  const configPath = join(dir, 'config.jsonc');
  writeFileSync(configPath, configSource);
  return spawnSync(process.execPath, [scriptPath, configPath], { encoding: 'utf8' });
}

test('guard runs when invoked through a file symlink (metered config fails)', () => {
  const dir = mkdtempSync(join(tmpdir(), 'metered-guard-link-'));
  const linkPath = join(dir, 'guard-link.mjs');
  symlinkSync(guardPath, linkPath);
  const res = runGuard(linkPath, meteredConfig);
  assert.equal(res.status, 1);
  assert.match(res.stderr, /Metered-provider config guard failed/);
  assert.match(res.stderr, /deepseek/);
});

test('guard runs when invoked through a file symlink (clean config passes)', () => {
  const dir = mkdtempSync(join(tmpdir(), 'metered-guard-link-'));
  const linkPath = join(dir, 'guard-link.mjs');
  symlinkSync(guardPath, linkPath);
  const res = runGuard(linkPath, cleanCliConfig);
  assert.equal(res.status, 0);
  assert.match(res.stdout, /Metered-provider config guard passed/);
});

test('guard runs when invoked through a symlinked directory (metered config fails)', () => {
  const dir = mkdtempSync(join(tmpdir(), 'metered-guard-dirlink-'));
  const linkDir = join(dir, 'bin-link');
  symlinkSync(dirname(guardPath), linkDir, 'dir');
  const res = runGuard(join(linkDir, 'check-opencode-metered-providers.mjs'), meteredConfig);
  assert.equal(res.status, 1);
  assert.match(res.stderr, /Metered-provider config guard failed/);
});

test('extra arguments are a usage error, not silently ignored (#7751)', () => {
  const dir = mkdtempSync(join(tmpdir(), 'metered-guard-argv-'));
  const goodPath = join(dir, 'good.jsonc');
  const badPath = join(dir, 'bad.jsonc');
  writeFileSync(goodPath, cleanCliConfig);
  writeFileSync(badPath, meteredConfig);
  const run = (argv) => spawnSync(process.execPath, [guardPath, ...argv], { encoding: 'utf8' });

  // Controls: a single config is checked exactly as before.
  assert.equal(run([goodPath]).status, 0);
  const badOnly = run([badPath]);
  assert.equal(badOnly.status, 1);
  assert.match(badOnly.stderr, /deepseek/);

  // A second positional is never silently dropped: the failing config
  // can no longer hide behind a passing first argument.
  const extra = run([goodPath, badPath]);
  assert.equal(extra.status, 2);
  assert.match(extra.stderr, /unexpected extra argument/);
  assert.match(extra.stderr, /bad\.jsonc/);

  // An unknown flag is rejected the same way, before any file is read.
  const flag = run([goodPath, '--bogus']);
  assert.equal(flag.status, 2);
  assert.match(flag.stderr, /--bogus/);
});

test('importing the guard module does not run main()', () => {
  const res = spawnSync(
    process.execPath,
    ['--input-type=module', '-e', `import ${JSON.stringify(pathToFileURL(guardPath).href)};`],
    { encoding: 'utf8' },
  );
  assert.equal(res.status, 0);
  assert.doesNotMatch(res.stdout + res.stderr, /Metered-provider config guard/);
});
