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

test('canonical OpenCode config passes the static guard', async () => {
  const { readFile } = await import('node:fs/promises');
  const { resolve } = await import('node:path');
  const config = await readFile(resolve('.opencode/opencode.jsonc'), 'utf8');
  assert.deepEqual(checkConfig(config), []);
});
