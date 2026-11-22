// CLI contract (fixed order, no aliases or --name=value):
//   --help
//   --config ABSOLUTE_PATH queue-list --limit N [--status STATUS]
//   --config ABSOLUTE_PATH queue-health|laptop-health|laptop-diagnostics
// Parse every argument before config reads, key import or network activity.
// The approved caller config is owner-only; device is fixed to amd. No key is
// generated, enrolled or printed, and no command/URL/path reaches the laptop.
import { isAbsolute } from 'node:path';
import { pathToFileURL } from 'node:url';
import { createCaller } from './caller.mjs';
import { readConfig, requireNonRoot } from './client.mjs';
import { identifier, statuses, validateOperation, validateResult } from './protocol.mjs';

export const USAGE = 'Usage: node aster.mjs --config ABSOLUTE_PATH queue-list --limit N [--status STATUS]\n'
  + '       node aster.mjs --config ABSOLUTE_PATH queue-health|laptop-health|laptop-diagnostics\n'
  + '       node aster.mjs --help\n';
const COMMANDS = Object.freeze({
  'queue-list': { operation: 'fleet.queue.list', method: 'fleetQueueList' },
  'queue-health': { operation: 'fleet.queue.health', method: 'fleetQueueHealth' },
  'laptop-health': { operation: 'laptop.health', method: 'laptopHealth' },
  'laptop-diagnostics': { operation: 'laptop.diagnostics', method: 'laptopDiagnostics' },
});
const SAFE_ERRORS = new Set(['non-root-required', 'invalid-config', 'invalid-operation', 'invalid-response',
  'invalid-handle', 'denied', 'quota', 'unavailable', 'timeout', 'cancelled', 'expired', 'stale-result',
  'submit-outcome-unknown']);

export function parseAsterArgs(argv) {
  if (!Array.isArray(argv) || !argv.every(v => typeof v === 'string')) throw new Error('usage');
  if (argv.length === 1 && argv[0] === '--help') return { help: true };
  if (argv.length < 3 || argv[0] !== '--config' || !isAbsolute(argv[1]) || argv[1].includes('\0')
    || !Object.hasOwn(COMMANDS, argv[2])) throw new Error('usage');
  const command = argv[2];
  let args = {};
  if (command === 'queue-list') {
    if (![5, 7].includes(argv.length) || argv[3] !== '--limit' || !/^(?:[1-9]|1[0-9]|20)$/.test(argv[4])) throw new Error('usage');
    args = { limit: Number(argv[4]) };
    if (argv.length === 7) {
      if (argv[5] !== '--status' || !statuses.includes(argv[6])) throw new Error('usage');
      args.status = argv[6];
    }
  } else if (argv.length !== 3) throw new Error('usage');
  const { operation, method } = COMMANDS[command];
  if (!validateOperation(operation, args)) throw new Error('usage');
  return { config: argv[1], command, operation, method, args };
}

const importSigningKey = jwk => crypto.subtle.importKey('jwk', jwk, { name: 'Ed25519' }, false, ['sign']);

/** All injected dependencies are for offline tests/in-process integration; none
 * are configurable through argv or the remote protocol. Returns a process code. */
export async function runAster(argv, { loadConfig = readConfig, importKey = importSigningKey, callerFactory = createCaller,
  getuid = process.getuid, stdout = text => process.stdout.write(text), stderr = text => process.stderr.write(text), signal } = {}) {
  let command;
  try { command = parseAsterArgs(argv); }
  catch { stderr(USAGE); return 2; }
  if (command.help) { stdout(USAGE); return 0; }
  const emit = value => {
    const line = JSON.stringify(value) + '\n';
    if (Buffer.byteLength(line, 'utf8') > 8192) throw new Error('invalid-response');
    stdout(line);
  };
  try {
    requireNonRoot(getuid);
    const config = await loadConfig(command.config, getuid);
    const privateKey = await importKey(config.privateKey);
    const caller = callerFactory({ origin: config.origin, audience: config.audience, principal: config.principal, privateKey, device: 'amd' });
    const deadline = signal ? AbortSignal.any([signal, AbortSignal.timeout(65000)]) : AbortSignal.timeout(65000);
    const options = { signal: deadline };
    const handle = command.command === 'queue-list'
      ? await caller[command.method](command.args, options)
      : await caller[command.method](options);
    // Only projected handle fields leave this process. expiresAt is a deadline
    // on this caller runner's clock, not a server UTC timestamp.
    if (!handle || !identifier(handle.id) || handle.operation !== command.operation || !Number.isSafeInteger(handle.expiresAt)) throw new Error('invalid-response');
    emit({ event: 'accepted', handle: { id: handle.id, operation: command.operation, expiresAt: handle.expiresAt, args: command.args } });
    const reply = await caller.wait(handle, options);
    if (reply?.status !== 'complete' || !validateResult(command.operation, reply.result)) throw new Error('invalid-response');
    if (command.command === 'queue-list' && reply.result.ok && (reply.result.data.tasks.length > command.args.limit
      || (command.args.status && reply.result.data.tasks.some(task => task.status !== command.args.status)))) throw new Error('invalid-response');
    emit({ event: 'complete', id: handle.id, operation: command.operation, result: reply.result });
    return reply.result.ok ? 0 : 1;
  } catch (error) {
    const candidate = error?.code ?? error?.message;
    const code = SAFE_ERRORS.has(candidate) ? candidate : 'unavailable';
    // No raw exception text, caller config, private key, response or stack.
    stderr(JSON.stringify({ event: 'error', error: code }) + '\n');
    return 1;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const controller = new AbortController();
  const stop = () => controller.abort();
  process.once('SIGINT', stop); process.once('SIGTERM', stop);
  runAster(process.argv.slice(2), { signal: controller.signal })
    .then(code => { process.exitCode = code; })
    .catch(() => { process.stderr.write('{"event":"error","error":"unavailable"}\n'); process.exitCode = 1; })
    .finally(() => { process.removeListener('SIGINT', stop); process.removeListener('SIGTERM', stop); });
}
