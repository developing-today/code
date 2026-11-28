---
session: ses_2c78
updated: 2026-03-29T09:40:43.220Z
---



## Conversation Summary

### Task
Build an OpenCode plugin (`opencode-auto-continue`) that detects HTTP 400 "bad request" errors and auto-sends "continue" to recover sessions. Add `/auto-continue` slash command for runtime configuration management.

### Accomplishments

**Plugin Built & Pushed** to `github.com/developing-today/opencode-auto-continue`:
- **Error detection**: Listens on `session.error`, `message.updated`, sends continue on `session.idle`
- **Safety**: Configurable cooldown (5s), delay (2s), max consecutive retries (5), auto-reset on success
- **`/auto-continue` command** (DCP pattern: `config` hook + `command.execute.before`):
  - `help`, `on/off`, `cooldown/delay/max <n>`, `status`, `reset` (session-level)
  - `global on/off/cooldown/delay/max` (writes `opencode-auto-continue.jsonc`)
  - `global update` (fetches latest SHA from GitHub API, pins in `opencode.jsonc`, clears bun cache)
- **README** updated: config optional, all commands documented
- **GitHub Action** `.github/workflows/tag-latest.yml`: auto-pushes `latest` and `@latest` tags on push to main
- Latest commit: `039384a` on `origin/main`

### Critical Blocker: Plugin Won't Load in OpenCode

OpenCode's embedded bun (1.2.27, bun 1.3.10) **cannot resolve GitHub references**. Every format tried:

| Config Format                                  | Install Step                  | Load Step                           | Result                    |
| ---------------------------------------------- | ----------------------------- | ----------------------------------- | ------------------------- |
| `github:developing-today/opencode-auto-continue` | ❌ appends `@latest`, bun fails | —                                   | `@latest failed to resolve` |
| `github:...#latest`                              | ❌ becomes `#latest@latest`     | —                                   | 404                       |
| `github:...#`                                    | ✅ `#@latest` resolves tag      | ❌ `require('github:...')` wrong path | Module not found          |
| `opencode-auto-continue@git+https://...`         | ❌ embedded bun fails         | —                                   | Code 1                    |
| `opencode-auto-continue@github:...`              | ❌ bun URL-encodes `/` as `%2f`   | —                                   | `InvalidURL`                |

**Root cause**: OpenCode's embedded bun URL-encodes `/` in github references (`developing-today%2fopencode-auto-continue`). System bun 1.3.10 handles all formats fine. This is an OpenCode bug.

**Pre-install workaround also fails**: `bun add github:...` in `~/.cache/opencode/` hits the same URL-encoding bug because the lockfile has a `@latest` resolution for existing packages.

### Key Files
- **Plugin source**: `/home/user/opencode-auto-continue/src/index.ts`
- **Plugin repo**: `github.com/developing-today/opencode-auto-continue`
- **OpenCode config**: `/home/user/code/.opencode/opencode.jsonc` (currently has `"opencode-auto-continue@github:developing-today/opencode-auto-continue"`)

### User Constraints
- **Must work from `opencode.jsonc` alone** — no manual cache manipulation
- **Never amend commits** — only forward commits
- **Never use reset/revert** — only go forward
- Git remote: `github.com/developing-today/code`

### Remaining Work
1. **Resolve the loading issue** — either publish to npm, or find a config format that works with OpenCode's embedded bun
2. Commit `.opencode/opencode.jsonc` changes to this repo once working
3. Update `global update` command if the config format changes
