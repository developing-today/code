#!/usr/bin/env node

import { realpathSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const DEFAULT_CONFIG = '.opencode/opencode.jsonc';

function lineStartsFor(source) {
  const starts = [0];
  for (let i = 0; i < source.length; i += 1) {
    if (source[i] === '\r') {
      if (source[i + 1] === '\n') i += 1;
      starts.push(i + 1);
    } else if (source[i] === '\n') {
      starts.push(i + 1);
    }
  }
  return starts;
}

function lineAt(position, starts) {
  let low = 0;
  let high = starts.length;
  while (low + 1 < high) {
    const mid = Math.floor((low + high) / 2);
    if (starts[mid] <= position) low = mid;
    else high = mid;
  }
  return low;
}

export function parseJsoncTree(source) {
  let i = 0;
  const comments = [];
  const lineStarts = lineStartsFor(source);

  const fail = (message) => {
    throw new SyntaxError(`${message} at offset ${i}`);
  };

  const skipTrivia = () => {
    for (;;) {
      while (i < source.length && /\s/.test(source[i])) i += 1;
      if (source.startsWith('//', i)) {
        const start = i;
        i += 2;
        while (i < source.length && source[i] !== '\n' && source[i] !== '\r') i += 1;
        comments.push({ kind: 'line', start, end: i, text: source.slice(start + 2, i) });
      } else if (source.startsWith('/*', i)) {
        const start = i;
        const close = source.indexOf('*/', i + 2);
        if (close < 0) fail('unterminated block comment');
        i = close + 2;
        comments.push({ kind: 'block', start, end: i, text: source.slice(start + 2, close) });
      } else {
        return;
      }
    }
  };

  const readString = () => {
    const start = i;
    if (source[i] !== '"') fail('expected string');
    i += 1;
    while (i < source.length) {
      if (source[i] === '\\') {
        i += 2;
      } else if (source[i] === '"') {
        i += 1;
        return { type: 'string', value: JSON.parse(source.slice(start, i)), start, end: i };
      } else {
        i += 1;
      }
    }
    fail('unterminated string');
  };

  const readValue = () => {
    skipTrivia();
    const start = i;
    if (i >= source.length) fail('expected value');

    if (source[i] === '"') return readString();

    if (source[i] === '{') {
      i += 1;
      const properties = [];
      skipTrivia();
      if (source[i] === '}') {
        i += 1;
        return { type: 'object', properties, start, end: i };
      }
      for (;;) {
        skipTrivia();
        if (source[i] !== '"') fail('expected object key');
        const key = readString();
        skipTrivia();
        if (source[i] !== ':') fail('expected colon');
        i += 1;
        const value = readValue();
        properties.push({ key: key.value, keyStart: key.start, value });
        skipTrivia();
        if (source[i] === '}') {
          i += 1;
          return { type: 'object', properties, start, end: i };
        }
        if (source[i] !== ',') fail('expected comma or closing brace');
        i += 1;
        skipTrivia();
        if (source[i] === '}') {
          i += 1;
          return { type: 'object', properties, start, end: i };
        }
      }
    }

    if (source[i] === '[') {
      i += 1;
      const items = [];
      skipTrivia();
      if (source[i] === ']') {
        i += 1;
        return { type: 'array', items, start, end: i };
      }
      for (;;) {
        items.push(readValue());
        skipTrivia();
        if (source[i] === ']') {
          i += 1;
          return { type: 'array', items, start, end: i };
        }
        if (source[i] !== ',') fail('expected comma or closing bracket');
        i += 1;
        skipTrivia();
        if (source[i] === ']') {
          i += 1;
          return { type: 'array', items, start, end: i };
        }
      }
    }

    const match = source.slice(i).match(/^(?:true|false|null|-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?)/);
    if (!match) fail('expected JSON value');
    i += match[0].length;
    return { type: 'primitive', value: JSON.parse(match[0]), start, end: i };
  };

  const root = readValue();
  skipTrivia();
  if (i !== source.length) fail('unexpected trailing content');
  return { root, comments, lineStarts };
}

function property(node, name) {
  if (node?.type !== 'object') return undefined;
  // Match JSON.parse behavior for duplicate keys: the last value wins.
  for (let i = node.properties.length - 1; i >= 0; i -= 1) {
    if (node.properties[i].key === name) return node.properties[i].value;
  }
  return undefined;
}

function effectiveProperties(node) {
  if (node?.type !== 'object') return [];
  const latest = new Map();
  node.properties.forEach((entry) => latest.set(entry.key, entry));
  return [...latest.values()].sort((left, right) => left.keyStart - right.keyStart);
}

function isMeteredModel(value) {
  if (typeof value !== 'string') return false;
  const provider = value.split('/', 1)[0];
  return provider === 'deepseek' || provider.startsWith('cloudflare-');
}

export function inspectConfig(source) {
  const { root, comments, lineStarts } = parseJsoncTree(source);
  const references = [];

  const addReference = (node, path, kind = 'model') => {
    if (node?.type !== 'string') return;
    references.push({
      value: node.value,
      path,
      kind,
      line: lineAt(node.start, lineStarts),
      provider: node.value.split('/', 1)[0],
      metered: isMeteredModel(node.value),
    });
  };

  const collectSelections = (node, path, kind = 'model') => {
    if (!node) return;
    if (node.type === 'string') {
      addReference(node, path, kind);
    } else if (node.type === 'array') {
      node.items.forEach((item, index) => collectSelections(item, `${path}[${index}]`, kind));
    } else if (node.type === 'object') {
      effectiveProperties(node).forEach(({ key, value }) => {
        if (['model', 'target', 'targets', 'fallback', 'fallbacks'].includes(key)) {
          collectSelections(value, `${path}.${key}`, kind);
        }
      });
    }
  };

  addReference(property(root, 'model'), '$.model');
  addReference(property(root, 'small_model'), '$.small_model');

  const walkAgents = (node, path) => {
    if (node?.type === 'array') {
      node.items.forEach((item, index) => walkAgents(item, `${path}[${index}]`));
    } else if (node?.type === 'object') {
      effectiveProperties(node).forEach(({ key, value }) => {
        const childPath = `${path}.${key}`;
        if (['model', 'small_model', 'fallback', 'fallbacks'].includes(key)) {
          collectSelections(value, childPath);
        } else {
          walkAgents(value, childPath);
        }
      });
    }
  };
  walkAgents(property(root, 'agent'), '$.agent');

  // The route map's keys are route NAMES, not definition fields: a route
  // named "model" (or "targets") is still a route. Iterate map entries as
  // names first, then walk each value as a route definition.
  const walkRouteDefinition = (node, path) => {
    if (node?.type === 'array') {
      node.items.forEach((item, index) => walkRouteDefinition(item, `${path}[${index}]`));
    } else if (node?.type === 'object') {
      effectiveProperties(node).forEach(({ key, value }) => {
        const childPath = `${path}.${key}`;
        if (key === 'targets' || key === 'fallback' || key === 'fallbacks') {
          collectSelections(value, childPath);
        } else if (key === 'model') {
          addReference(value, childPath);
        } else {
          walkRouteDefinition(value, childPath);
        }
      });
    }
  };

  const walkModelRoutes = (node, path) => {
    if (node?.type === 'array') {
      node.items.forEach((item, index) => walkModelRoutes(item, `${path}[${index}]`));
    } else if (node?.type === 'object') {
      effectiveProperties(node).forEach(({ key, value }) => {
        walkRouteDefinition(value, `${path}.${key}`);
      });
    }
  };

  const experimental = property(root, 'experimental');
  // Mirror the route proxy's fallback (bin/opencode-route-proxy.mjs): the
  // experimental map wins when present, otherwise the top-level map is the
  // active runtime input and must be guarded too.
  const modelRoutes = property(experimental, 'model_routes');
  if (modelRoutes?.type === 'object' || modelRoutes?.type === 'array') {
    walkModelRoutes(modelRoutes, '$.experimental.model_routes');
  } else {
    walkModelRoutes(property(root, 'model_routes'), '$.model_routes');
  }

  // Provider-route group names are not definition fields either: a group
  // named "providers" is still a group. Iterate map entries as names
  // first, then walk each value as a group definition.
  const walkProviderGroup = (node, path) => {
    if (node?.type === 'array') {
      node.items.forEach((item, index) => walkProviderGroup(item, `${path}[${index}]`));
    } else if (node?.type === 'object') {
      effectiveProperties(node).forEach(({ key, value }) => {
        const childPath = `${path}.${key}`;
        if (key === 'providers') collectSelections(value, childPath, 'provider-route');
        else walkProviderGroup(value, childPath);
      });
    }
  };

  const walkProviderRoutes = (node, path) => {
    if (node?.type === 'array') {
      node.items.forEach((item, index) => walkProviderRoutes(item, `${path}[${index}]`));
    } else if (node?.type === 'object') {
      effectiveProperties(node).forEach(({ key, value }) => {
        walkProviderGroup(value, `${path}.${key}`);
      });
    }
  };
  const providerRoutes = property(experimental, 'provider_routes');
  if (providerRoutes?.type === 'object' || providerRoutes?.type === 'array') {
    walkProviderRoutes(providerRoutes, '$.experimental.provider_routes');
  } else {
    walkProviderRoutes(property(root, 'provider_routes'), '$.provider_routes');
  }

  const markerComments = comments.filter((comment) => /payg-exception/i.test(comment.text));
  const markers = markerComments.map((comment) => {
    const line = lineAt(comment.start, lineStarts);
    const lineStart = lineStarts[line];
    const lineEnd = line + 1 < lineStarts.length ? lineStarts[line + 1] - 1 : source.length;
    const fullLine = source.slice(lineStart, lineEnd).trim();
    const match = comment.kind === 'line'
      ? fullLine.match(/^\/\/\s*payg-exception:\s*(\S.*?)\s*$/i)
      : null;
    const reason = match?.[1]?.trim() ?? '';
    // The marker is per-use: even a blank line or intervening comment breaks
    // adjacency, so the next physical line must contain exactly one selection.
    const selectedLine = line + 1 < lineStarts.length ? line + 1 : -1;
    const attached = references.filter((reference) => reference.line === selectedLine);
    return { line, reason, attached };
  });

  const issues = [];
  const acceptedMarkerReferences = new Set();
  for (const marker of markers) {
    const lineNumber = marker.line + 1;
    if (!marker.reason) {
      issues.push(`payg-exception marker at line ${lineNumber} needs a non-empty reason`);
    } else if (marker.attached.length !== 1) {
      issues.push(`payg-exception marker at line ${lineNumber} must be immediately above exactly one model/provider selection`);
    } else if (!marker.attached[0].metered) {
      issues.push(`payg-exception marker at line ${lineNumber} is attached to a non-metered selection`);
    } else {
      acceptedMarkerReferences.add(marker.attached[0]);
    }
  }

  for (const reference of references) {
    if (!reference.metered || acceptedMarkerReferences.has(reference)) continue;
    const selectionType = reference.kind === 'provider-route' ? 'provider-route selection' : 'model selection';
    issues.push(
      `metered provider "${reference.provider}" selected by ${reference.path} (${reference.value}; ${selectionType}); add an adjacent // payg-exception: <reason> only for an explicitly approved use`,
    );
  }

  return { references, issues };
}

export function checkConfig(source) {
  return inspectConfig(source).issues;
}

async function main() {
  const configPath = resolve(process.argv[2] ?? DEFAULT_CONFIG);
  const source = await readFile(configPath, 'utf8');
  const issues = checkConfig(source);
  if (issues.length) {
    console.error(`Metered-provider config guard failed for ${configPath}:`);
    for (const issue of issues) console.error(`- ${issue}`);
    process.exitCode = 1;
    return;
  }
  console.log(`Metered-provider config guard passed: ${configPath}`);
}

// Entry-point check: compare realpaths, not the raw strings (#4694;
// same pattern as proxy-supervise.mjs / #2083 and
// provider-catalog-sync.mjs / #2235). process.argv[1] keeps the path
// exactly as invoked (e.g. through a file or directory symlink) while
// import.meta.url is the resolved module URL, so a resolve() string
// compare silently never matches and main() never runs via a symlink --
// the guard exited 0 without checking anything.
const isMain = (() => {
  try {
    if (!process.argv[1]) return false;
    return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
})();

if (isMain) {
  await main();
}
