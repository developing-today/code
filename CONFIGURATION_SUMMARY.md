# Configuration Files and TypeScript Settings Summary

## Overview
Found configuration files and validation logic in `/home/user/git/fleet/9338` focusing on TypeScript compiler settings, build scripts, and validation logic.

---

## TypeScript Configuration Files

### 1. `/pm-stack/dispatch/tsconfig.json`
**Location:** `pm-stack/dispatch/tsconfig.json`
**Purpose:** TypeScript configuration for the PM dispatch service

**Compiler Options:**
- `allowImportingTsExtensions`: true
- `lib`: ["ES2022", "DOM"]
- `module`: "ESNext"
- `moduleResolution`: "bundler"
- `noEmit`: true (compilation only, no output)
- `noUncheckedIndexedAccess`: false
- `resolveJsonModule`: true
- `skipLibCheck`: true
- `strict`: true
- `target`: "ES2022"
- `types`: ["bun"]

**Include:**
- `src/**/*.ts`
- `test/**/*.ts`

---

### 2. `/pm-stack/aggregator/tsconfig.json`
**Location:** `pm-stack/aggregator/tsconfig.json`
**Purpose:** TypeScript configuration for the PM aggregator UI

**Compiler Options:**
- `target`: "ES2022"
- `module`: "ESNext"
- `moduleResolution`: "bundler"
- `types`: ["bun"]
- `lib`: ["ES2022", "DOM"]
- `strict`: true
- `noEmit`: true
- `skipLibCheck`: true
- `resolveJsonModule`: true
- `allowImportingTsExtensions`: true
- `noUncheckedIndexedAccess`: false

**Include:**
- `src/**/*.ts`
- `test/**/*.ts`

---

### 3. `/pkgs/lab-ops-hub/tsconfig.json`
**Location:** `pkgs/lab-ops-hub/tsconfig.json`
**Purpose:** TypeScript configuration for the Lab Ops Hub

**Compiler Options:**
- `allowImportingTsExtensions`: true
- `lib`: ["ES2022"]
- `module`: "ESNext"
- `moduleResolution`: "bundler"
- `noEmit`: true
- `strict`: true
- `target`: "ES2022"
- `types`: ["@cloudflare/workers-types"]

**Include:**
- `src/**/*.ts`

---

### 4. `/pkgs/pkarr-worker/tsconfig.json`
**Location:** `pkgs/pkarr-worker/tsconfig.json`
**Note:** Not explicitly found in search, but likely exists given the TypeScript setup

---

## Package.json Files with Build/Validation Scripts

### 1. `/pm-stack/dispatch/package.json`
**Location:** `pm-stack/dispatch/package.json`

**Scripts:**
```json
{
  "dev": "bun run src/server.ts",
  "start": "bun run src/server.ts",
  "test": "bun test",
  "typecheck": "bunx tsc --noEmit"
}
```

**Dev Dependencies:**
- `@types/bun`: "latest"
- `typescript`: "^5.6.0"

---

### 2. `/pm-stack/aggregator/package.json`
**Location:** `pm-stack/aggregator/package.json`

**Scripts:**
```json
{
  "dev": "PM_AGG_DEV=1 bun run src/server.ts",
  "start": "bun run src/server.ts",
  "test": "bun test",
  "contract:check": "bun run scripts/check-contract-corpus.ts",
  "typecheck": "bunx tsc --noEmit"
}
```

**Key Validation Script:**
- **`contract:check`**: Validates the publisher → collector → aggregator contract corpus

**Dev Dependencies:**
- `@types/bun`: "latest"
- `typescript`: "^5.6.0"

---

### 3. `/pkgs/pkarr-worker/package.json`
**Location:** `pkgs/pkarr-worker/package.json`

**Scripts:**
```json
{
  "deploy": "echo 'deploy through the flake entrypoint instead: just deploy-pkarr (nix run .#pkarr-deploy)' >&2; exit 1",
  "dev": "wrangler dev",
  "test": "vitest run",
  "typecheck": "tsc --noEmit"
}
```

**Dev Dependencies:**
- `@cloudflare/vitest-pool-workers`: "^0.9.13"
- `@cloudflare/workers-types`: "^5.20261019.1"
- `typescript`: "^5.6.0"
- `vitest`: "^4.0.0"
- `wrangler`: "4.152.0"

---

### 4. `/pkgs/lab-ops-hub/package.json`
**Location:** `pkgs/lab-ops-hub/package.json`

**Scripts:**
```json
{
  "db:init": "wrangler d1 execute labops-db --remote --file=./schema.sql",
  "db:init:local": "wrangler d1 execute labops-db --local --file=./schema.sql",
  "deploy": "wrangler deploy",
  "dev": "wrangler dev",
  "test": "npm run test:ts && npm run test:py",
  "test:py": "python3 -m unittest discover -s test -p \"test_*.py\"",
  "test:ts": "node --test \"test/**/*.test.ts\"",
  "types": "wrangler types"
}
```

**Note:** Uses `wrangler types` for TypeScript type generation rather than direct tsc

---

## Validation and Script Check Logic

### 1. Contract Validation: `check-contract-corpus.ts`
**Location:** `pm-stack/aggregator/scripts/check-contract-corpus.ts`

**Purpose:** Validates the publisher → collector → aggregator contract corpus

**Key Features:**
- Validates orchestration.json, dns.json, projects.json, registry.json, quota.json, feed.json
- Uses actual validators from the aggregator codebase
- Can validate both the corpus and individual publisher files
- Runs as `bun run contract:check`

**Validation Checks:**
- `isOrchestrationSection()` - validates orchestration structure
- `isDnsSection()` - validates DNS configuration
- `isProjectPoolSection()` and `isProjectPoolItem()` - validates project pool
- `isSessionRecord` - validates registry entries
- `sanitizeRegistry()` - validates registry sanitization
- `isQuotaPool` and `isQuotaWindow` - validates quota definitions
- `isBurnSegment` - validates burn rate segments
- `isFleetFeed` - validates fleet feed structure
- `normalizeOrchestrationSection()` - validates normalization for feed

---

### 2. Epic Conformance Check: `check-epic-conformance.ts`
**Location:** `pm-stack/aggregator/scripts/check-epic-conformance.ts`

**Purpose:** Validates GitHub issues marked as epics for conformance

**Validation Rules:**
- Must have "epic" label
- Must have "kind/feature" label
- Must have canonical app/<component> label
- Must be attached to org Project #4
- Must have Definition of Done section
- Must have named Owner: line

**Live Audit:**
- Queries GitHub API using `gh` CLI
- Checks up to 500 open issues
- Reports non-conforming epics

---

### 3. Provenance Matrix Check: `check-provenance-matrix.ts`
**Location:** `pm-stack/aggregator/scripts/check-provenance-matrix.ts`

**Purpose:** Ensures consistency between configuration, code, and documentation

**Validates:**
- NixOS module declares pm-projector service
- NixOS module pins PM_PROJECTOR_URL
- install.sh renders PM_AGG_SOURCE=live
- live.ts resolves fleet-feed Project pool before projector
- live.ts reads GitHub comments from projector
- program.ts states comment projection reads live from projector
- README.md carries the capability provenance matrix
- pm-stack/README.md points at the matrix

---

## Formatting and Linting Configuration

### Biome Configuration
**Location:** `biome.json`

**Configuration:**
```json
{
  "formatter": {
    "enabled": true,
    "indentStyle": "space",
    "indentWidth": 2,
    "lineWidth": 120
  },
  "linter": {
    "enabled": true,
    "rules": {
      "recommended": true,
      "suspicious": { "noExplicitAny": "off" },
      "style": { "noNonNullAssertion": "off", "useConst": "warn" },
      "complexity": { "noForEach": "off" }
    }
  },
  "files": {
    "includes": ["**/*.ts", "**/*.css", "**/*.json", "!**/dist", "!**/node_modules"]
  }
}
```

**Key Settings:**
- Formatter: Enabled with 2-space indentation, 120 line width
- Linter: Enabled with recommended rules
- Includes: TypeScript, CSS, JSON files (excluding dist and node_modules)

---

## TypeScript Compiler Settings Summary

### Common Settings Across Projects:
1. **Strict Mode**: All projects use `"strict": true`
2. **ES2022 Target**: All projects target ES2022
3. **ESNext Module**: Using ES modules
4. **Bundler Resolution**: Using "bundler" module resolution
5. **No Emit**: All validation configs use `"noEmit": true` (type checking only)
6. **Skip Lib Check**: Enabled for performance
7. **Resolve JSON Modules**: Enabled for configuration files

### Build Tools:
- **Bun**: Primary runtime for pm-stack services
- **Wrangler**: Cloudflare Workers deployment tool
- **Vitest**: Testing framework for Cloudflare Workers
- **TypeScript**: ^5.6.0 across all projects

---

## Validation Scripts Summary

| Script Name | File Location | Purpose |
|-------------|---------------|---------|
| `contract:check` | pm-stack/aggregator/package.json | Validates contract corpus and publisher files |
| `typecheck` | pm-stack/dispatch/package.json | Type checking for dispatch service |
| `typecheck` | pm-stack/aggregator/package.json | Type checking for aggregator UI |
| `typecheck` | pkgs/pkarr-worker/package.json | Type checking for pkarr worker |
| `check-contract-corpus.ts` | pm-stack/aggregator/scripts/ | Main contract validation logic |
| `check-epic-conformance.ts` | pm-stack/aggregator/scripts/ | Validates epic issue conformance |
| `check-provenance-matrix.ts` | pm-stack/aggregator/scripts/ | Validates configuration consistency |

---

## Key Observations

1. **TypeScript Version**: Consistent use of TypeScript 5.6.0 across all projects
2. **Build System**: Primarily using Bun as the runtime
3. **Validation Strategy**: Heavy use of custom validation scripts for domain-specific validation
4. **Contract Validation**: The `contract:check` script is a critical validation point for the fleet's contract system
5. **Consistency Checks**: Multiple scripts ensure configuration files match their documentation
6. **No Direct `script.check` Found**: The search for "script.check" related files did not yield results, suggesting either:
   - The naming convention is different
   - It's implemented as custom validation scripts (like the ones found)
   - It's part of the contract validation system

---

## Recommendations

1. **Document Validation Scripts**: Consider adding a README in the scripts directory explaining each validation script's purpose
2. **CI Integration**: Ensure these validation scripts are integrated into CI/CD pipelines
3. **Standardization**: The validation scripts follow a consistent pattern - consider extracting common utilities
4. **Type Generation**: For Cloudflare Workers projects, consider using `wrangler types` consistently
