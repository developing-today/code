# opencode-auto-continue

OpenCode plugin that automatically sends "continue" when bad request (HTTP 400) errors occur, allowing sessions to recover and resume without manual intervention.

## How It Works

1. **Detects errors**: Listens for `session.error` and `message.updated` events
2. **Identifies bad requests**: Checks for `ApiError` with status code 400 or messages containing "bad request"
3. **Waits for idle**: When the session becomes idle after an error, sends "continue" via `promptAsync`
4. **Safety limits**: Cooldown (5s) and max consecutive retries (5) prevent infinite loops

## Installation

### From GitHub (local file reference)

Add to your `.opencode/package.json` dependencies:

```json
{
  "dependencies": {
    "opencode-auto-continue": "file:../pkgs/opencode-auto-continue"
  }
}
```

Then add to your `.opencode/opencode.jsonc` plugin array:

```jsonc
{
  "plugin": [
    "opencode-auto-continue",
    // ... other plugins
  ]
}
```

Run `bun install` in your `.opencode/` directory.

### From npm (if published)

```bash
# In your .opencode/ directory
bun add opencode-auto-continue
```

Then add `"opencode-auto-continue"` to the `plugin` array in `.opencode/opencode.jsonc`.

## Building

```bash
cd pkgs/opencode-auto-continue
bun install
bun run build
```

## Configuration

Currently uses hardcoded defaults:

| Setting | Default | Description |
|---------|---------|-------------|
| `COOLDOWN_MS` | 5000 | Minimum ms between auto-continues per session |
| `DELAY_MS` | 2000 | Delay after session idle before sending continue |
| `MAX_CONSECUTIVE` | 5 | Max consecutive auto-continues before giving up |

## Logs

The plugin logs all activity to the console with `[opencode-auto-continue]` prefix:

```
[opencode-auto-continue] Bad request error in session abc123: Bad Request
[opencode-auto-continue] Session abc123 is idle with pending continue, waiting 2000ms...
[opencode-auto-continue] Sending "continue" to session abc123 (attempt 1/5)
[opencode-auto-continue] Successfully sent "continue" to session abc123
```

## License

MIT
