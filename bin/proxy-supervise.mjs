#!/usr/bin/env node
// proxy-supervise.mjs — hash-keyed A/B supervisor for the fleet proxies.
//
// Drew's requirement (2026-10-24): each server-based provider's default
// server is started by a service on the laptop / the init flow on the
// sandbox, and a NixOS rebuild + switch must NOT kill a running proxy
// unless the artifact that would run actually changed (hash). When it
// did change, the roll is graceful: the new build comes up on the
// alternate port while the old one keeps serving, health is verified,
// the endpoint record flips, and only then is the old instance drained.
//
// This supervisor is proxy-agnostic: a proxy is an opaque command plus a
// port env var, a /health endpoint, and a set of files whose content
// defines "the artifact". It never reaches inside a proxy.
//
// Port scheme: canonical port P (fleet map: 18742 clef, 18743 agy,
// 18744 codex, 18745 antigravity-ls, 18746 design-code, 18747 muse,
// 18748 commandcode) and alternate P + altPortOffset (default 1000:
// 19744, 19745, ...). At most two instances of a proxy ever run, one
// per port, and only during a roll.
//
// Strategies per proxy:
//   "ab"      — steady state lives on whichever port the last cutover
//               landed; a hash change rolls to the other port and stays
//               there. (Laptop proxies: provider wiring follows the
//               endpoint record / *_PROXY_PORT env, flipped by whoever
//               owns the serve; see the PR notes.)
//   "swap"    — a hash change is first VALIDATED on the alternate port;
//               once healthy, the candidate and the old instance stop
//               and the new build starts on the canonical port, so
//               wiring that hard-codes the canonical port (the sandbox
//               OpenCode config) never strands. Brief canonical outage
//               (~seconds) during the swap itself.
//   "inplace" — stop, start on the same port, health-check. For
//               stateless forwarders (the sandbox tailnet relays).
//
// State per proxy: <stateDir>/<name>.json — the supervisor's record of
// the current instance {port, hash, pid, startedAt, adopted}.
// The public endpoint record keeps the fleet location
// <endpointDir>/<name>/endpoint.json and is EXTENDED on cutover to
// {host, port, url, pid, hash, startedAt, previousPort,
//  serveRestartRequired, supervisor, updatedAt}. (A proxy overwrites it
// with its own minimal shape when IT starts; the supervisor rewrites
// the extended shape after health passes, and leaves it alone on a
// no-op reconcile apart from restoring the extension.)
//
// A foreign instance (started by systemd/home-manager, not by this
// supervisor) is ADOPTED when it is healthy and the artifact files are
// no newer than the process start (mtime heuristic); if the files are
// newer, the foreign instance is stale and is rolled like any other.
//
// Usage:
//   node proxy-supervise.mjs --config <config.json> [--only <name>]
//                            [--status] [--dry-run]
// Exit code: 0 when every selected proxy reconciled (or was already
// correct); 1 when any roll failed (the old instance is left running).
import { spawn, spawnSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';

// ---------- small utilities ----------

export const expandPath = (p, env = process.env) => {
  if (typeof p !== 'string') return p;
  let out = p.replace(/\$\{([A-Za-z_][A-Za-z0-9_]*)\}/g, (_, k) => env[k] ?? '');
  out = out.replace(/\$([A-Za-z_][A-Za-z0-9_]*)/g, (_, k) => env[k] ?? '');
  if (out === '~') return os.homedir();
  if (out.startsWith('~/')) return path.join(os.homedir(), out.slice(2));
  return out;
};

export const sha256Hex = (data) => crypto.createHash('sha256').update(data).digest('hex');

export const fileHash = (file) => {
  try {
    return sha256Hex(fs.readFileSync(file));
  } catch {
    return 'MISSING';
  }
};

export const pidAlive = (pid) => {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};

// Epoch ms at which a pid started, from /proc (Linux). null elsewhere.
export const pidStartedAt = (pid) => {
  try {
    const stat = fs.readFileSync(`/proc/${pid}/stat`, 'utf8');
    const startTicks = Number(stat.slice(stat.lastIndexOf(')') + 2).split(' ')[19]);
    const uptime = Number(fs.readFileSync('/proc/uptime', 'utf8').split(' ')[0]);
    const started = Date.now() - uptime * 1000 + (startTicks / 100) * 1000;
    return Math.round(started);
  } catch {
    return null;
  }
};

const readJson = (file) => {
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch {
    return null;
  }
};

const writeJsonAtomic = (file, obj) => {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.tmp-${process.pid}`;
  fs.writeFileSync(tmp, JSON.stringify(obj, null, 2) + '\n');
  fs.renameSync(tmp, file);
};

// ---------- config ----------

export const loadConfig = (file) => {
  const raw = JSON.parse(fs.readFileSync(file, 'utf8'));
  const cfg = {
    stateDir: expandPath(raw.stateDir || '~/.local/state/proxy-supervisor'),
    endpointDir: expandPath(raw.endpointDir || '~/.local/state'),
    // 'system' | 'user' | null. When set, instances are started as
    // transient systemd units (systemd-run) — see spawnInstance for
    // why plain detached children are not enough.
    systemdRun: raw.systemdRun || null,
    drainSeconds: raw.drainSeconds ?? 5,
    healthTimeoutSeconds: raw.healthTimeoutSeconds ?? 45,
    lockTimeoutSeconds: raw.lockTimeoutSeconds ?? 900,
    proxies: (raw.proxies || []).map((p) => ({
      altPortOffset: 1000,
      strategy: 'ab',
      host: '127.0.0.1',
      healthPath: '/health',
      hashFiles: [],
      hashEnv: [],
      env: {},
      backends: [],
      ...p,
    })),
  };
  for (const p of cfg.proxies) {
    p.command = p.command.map((a) => expandPath(a));
    p.hashFiles = p.hashFiles.map((f) => expandPath(f));
    if (p.cwd) p.cwd = expandPath(p.cwd);
  }
  return cfg;
};

// ---------- hashing ----------

// The hash of "the artifact that would run": the entrypoint + listed
// files' contents, the runtime binary's resolved path (a node rebuild
// changes the store path), selected env values (e.g. ANTIGRAVITY_LS_BIN
// — a language-server rebuild is an artifact change), and the current
// ports of declared backends (a backend cutover changes what this
// proxy would connect to, so it must roll too).
export const computeHash = (proxy, resolvedEnv, backendPorts) => {
  const parts = [];
  try {
    parts.push(['runtime', fs.realpathSync(proxy.command[0])]);
  } catch {
    parts.push(['runtime', proxy.command[0]]);
  }
  for (const f of proxy.hashFiles) parts.push(['file', f, fileHash(f)]);
  for (const k of proxy.hashEnv) parts.push(['env', k, resolvedEnv[k] ?? '']);
  for (const b of proxy.backends) parts.push(['backend', b.name, backendPorts[b.name] ?? null]);
  return sha256Hex(JSON.stringify(parts));
};

export const otherPort = (proxy, port) =>
  port === proxy.canonicalPort ? proxy.canonicalPort + proxy.altPortOffset : proxy.canonicalPort;

// Resolve the env an instance would run with (minus its own port,
// which is instance state, not artifact).
export const resolveEnv = (proxy, baseEnv, backendPorts) => {
  const env = { ...baseEnv };
  for (const [k, v] of Object.entries(proxy.env)) env[k] = expandPath(String(v), env);
  for (const b of proxy.backends) {
    if (backendPorts[b.name] != null) env[b.env] = String(backendPorts[b.name]);
  }
  return env;
};

// ---------- health ----------

export const checkHealth = (proxy, port, timeoutMs = 2000) =>
  new Promise((resolve) => {
    const req = http.get(
      { host: proxy.host, port, path: proxy.healthPath, timeout: timeoutMs },
      (res) => {
        let body = '';
        res.on('data', (c) => (body += c));
        res.on('end', () => {
          let ok = res.statusCode === 200;
          try {
            const j = JSON.parse(body);
            if (j && typeof j === 'object' && 'status' in j) ok = ok && j.status === 'ok';
          } catch { /* non-JSON 200 still counts */ }
          resolve({ ok, status: res.statusCode, body: body.slice(0, 200) });
        });
      }
    );
    req.on('timeout', () => { req.destroy(); resolve({ ok: false, status: 0, body: '' }); });
    req.on('error', () => resolve({ ok: false, status: 0, body: '' }));
  });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const waitHealthy = async (proxy, port, timeoutSeconds, isDead) => {
  const deadline = Date.now() + timeoutSeconds * 1000;
  for (;;) {
    const h = await checkHealth(proxy, port, 2000);
    if (h.ok) return true;
    if (isDead && isDead()) return false;
    if (Date.now() > deadline) return false;
    await sleep(400);
  }
};

// ---------- instance control ----------
//
// Instances are started as transient systemd units (systemd-run) when
// the config says so. A plain detached child is NOT enough: when the
// supervisor runs as a systemd oneshot (the timer), children it spawns
// land in the oneshot's cgroup and systemd kills them all when the
// oneshot exits — observed live on the sandbox (the timer's first
// reconcile started commandcode-proxy and the codex relay; both died
// with the invocation, while shell-started instances survived). A
// transient unit is its own cgroup and lifecycle: it survives the
// supervisor, a daemon-reload, and a nixos switch (switch only
// restarts units whose generated definitions changed), and stopping
// it takes the proxy's own children (e.g. commandcode's ACP process)
// with it. Unit name: fps-<proxy>-<port>.

const scArgs = (cfg) => (cfg.systemdRun === 'user' ? ['--user'] : []);
const systemctl = (cfg, args) =>
  spawnSync('systemctl', [...scArgs(cfg), ...args], { encoding: 'utf8', timeout: 30000 });

const unitNameFor = (proxy, port) => `fps-${proxy.name}-${port}`;

const spawnInstance = async (cfg, proxy, env, port) => {
  const fullEnv = { ...env, [proxy.portEnv]: String(port) };
  if (cfg.systemdRun) {
    const unit = unitNameFor(proxy, port);
    const setenv = Object.entries(fullEnv).flatMap(([k, v]) => [`--setenv=${k}=${v}`]);
    const run = () =>
      spawnSync('systemd-run', [
        ...scArgs(cfg), `--unit=${unit}`, '--collect',
        `--working-directory=${proxy.cwd || os.homedir()}`,
        ...setenv, '--', ...proxy.command,
      ], { encoding: 'utf8', timeout: 30000 });
    let r = run();
    if (r.status !== 0) {
      // A previous generation's unit may linger (deactivating/failed).
      systemctl(cfg, ['stop', unit]);
      systemctl(cfg, ['reset-failed', unit]);
      r = run();
    }
    if (r.status !== 0) {
      return { pid: null, unit, isDead: () => true, error: (r.stderr || 'systemd-run failed').slice(0, 200) };
    }
    // MainPID appears asynchronously.
    let pid = null;
    for (let i = 0; i < 25 && !pid; i++) {
      const out = systemctl(cfg, ['show', unit, '-p', 'MainPID', '--value']);
      const n = Number((out.stdout || '').trim());
      if (Number.isInteger(n) && n > 0) pid = n;
      else await sleep(200);
    }
    return { pid, unit, isDead: () => !pidAlive(pid) };
  }
  const logFile = path.join(cfg.stateDir, 'logs', `${proxy.name}-${port}.log`);
  fs.mkdirSync(path.dirname(logFile), { recursive: true });
  const fd = fs.openSync(logFile, 'a');
  const child = spawn(proxy.command[0], proxy.command.slice(1), {
    cwd: proxy.cwd || os.homedir(),
    env: fullEnv,
    detached: true,
    stdio: ['ignore', fd, fd],
  });
  fs.closeSync(fd);
  child.unref();
  let exited = false;
  child.on('exit', () => { exited = true; });
  return { pid: child.pid, unit: null, isDead: () => exited || !pidAlive(child.pid) };
};

export const stopInstance = async (pid, drainSeconds, cfg = null, unit = null) => {
  if (unit && cfg) systemctl(cfg, ['stop', unit]);
  if (!pidAlive(pid)) return 'already-dead';
  if (!unit) {
    try { process.kill(pid, 'SIGTERM'); } catch { return 'already-dead'; }
  }
  const deadline = Date.now() + drainSeconds * 1000;
  while (Date.now() < deadline) {
    if (!pidAlive(pid)) return unit ? 'unit-stop' : 'sigterm';
    await sleep(200);
  }
  try { process.kill(pid, 'SIGKILL'); } catch { /* gone */ }
  return 'sigkill';
};

// ---------- endpoint record ----------

const endpointFile = (cfg, name) => path.join(cfg.endpointDir, name, 'endpoint.json');

const writeEndpoint = (cfg, proxy, { port, pid, hash, startedAt, previousPort, unit }) => {
  writeJsonAtomic(endpointFile(cfg, proxy.name), {
    host: proxy.host,
    port,
    url: `http://${proxy.host}:${port}`,
    pid,
    unit: unit ?? null,
    hash,
    startedAt,
    previousPort: previousPort ?? null,
    // The proxies' OpenCode wiring resolves {env:<NAME>_PROXY_PORT:-P}
    // from the serve process env, fixed at serve start; a flip is only
    // picked up when that serve next starts (or its config is edited).
    // The endpoint record is the authority other tooling reads.
    serveRestartRequired: port !== proxy.canonicalPort,
    supervisor: 'proxy-supervise',
    updatedAt: new Date().toISOString(),
  });
};

// ---------- locking ----------

const acquireLock = (cfg, name, lockTimeoutSeconds) => {
  const file = path.join(cfg.stateDir, `${name}.lock`);
  fs.mkdirSync(cfg.stateDir, { recursive: true });
  try {
    const fd = fs.openSync(file, 'wx');
    fs.writeFileSync(fd, `${process.pid}\n`);
    fs.closeSync(fd);
    return file;
  } catch {
    let holder = null;
    try { holder = Number(fs.readFileSync(file, 'utf8').split('\n')[0]) || null; } catch { /* unreadable */ }
    let stale = false;
    try {
      const age = Date.now() - fs.statSync(file).mtimeMs;
      stale = !pidAlive(holder) || age > lockTimeoutSeconds * 1000;
    } catch { stale = true; }
    if (!stale) return null;
    try { fs.unlinkSync(file); } catch { /* raced */ }
    const fd = fs.openSync(file, 'wx');
    fs.writeFileSync(fd, `${process.pid}\n`);
    fs.closeSync(fd);
    return file;
  }
};

// ---------- reconcile ----------

const stateFile = (cfg, name) => path.join(cfg.stateDir, `${name}.json`);

export const currentBackendPorts = (cfg, proxy) => {
  const ports = {};
  for (const b of proxy.backends) {
    const ep = readJson(endpointFile(cfg, b.name));
    const st = readJson(stateFile(cfg, b.name));
    const backendCfg = cfg.proxies.find((p) => p.name === b.name);
    ports[b.name] = st?.current?.port ?? ep?.port ?? backendCfg?.canonicalPort ?? null;
  }
  return ports;
};

export const reconcile = async (cfg, proxy, { dryRun = false, log = console.log } = {}) => {
  const result = { name: proxy.name, action: 'none', port: null, hash: null, pid: null };
  const lock = dryRun ? null : acquireLock(cfg, proxy.name, cfg.lockTimeoutSeconds);
  if (!dryRun && lock === null) {
    result.action = 'locked';
    log(JSON.stringify(result));
    return result;
  }
  try {
    const state = readJson(stateFile(cfg, proxy.name)) || {};
    const endpoint = readJson(endpointFile(cfg, proxy.name));
    const backendPorts = currentBackendPorts(cfg, proxy);
    const env = resolveEnv(proxy, process.env, backendPorts);
    const hash = computeHash(proxy, env, backendPorts);
    result.hash = hash.slice(0, 12);

    const altPort = proxy.canonicalPort + proxy.altPortOffset;
    const ports = [proxy.canonicalPort, altPort];
    const health = {};
    for (const p of ports) health[p] = await checkHealth(proxy, p, 1500);

    // What is running, per our own record?
    let current = state.current || null;
    if (current && (!pidAlive(current.pid) || !health[current.port]?.ok)) current = { ...current, live: false };
    else if (current) current = { ...current, live: true };

    // A healthy instance we manage, already on the desired hash: the
    // no-op path — the NixOS-switch survival property lives here.
    if (current?.live && current.hash === hash && proxy.strategy !== 'swap') {
      result.action = 'noop';
      result.port = current.port;
      result.pid = current.pid;
      // Restore the extended endpoint shape if the proxy overwrote it.
      if (endpoint?.hash !== hash || endpoint?.port !== current.port) {
        if (!dryRun) writeEndpoint(cfg, proxy, current);
      }
      // Clean up a leftover previous instance (interrupted roll).
      if (state.previous && state.previous.pid !== current.pid && pidAlive(state.previous.pid)) {
        if (!dryRun) await stopInstance(state.previous.pid, cfg.drainSeconds, cfg, state.previous.unit);
      }
      log(JSON.stringify(result));
      return result;
    }
    if (current?.live && current.hash === hash && proxy.strategy === 'swap' && current.port === proxy.canonicalPort) {
      result.action = 'noop';
      result.port = current.port;
      result.pid = current.pid;
      if (endpoint?.hash !== hash && !dryRun) writeEndpoint(cfg, proxy, current);
      log(JSON.stringify(result));
      return result;
    }
    // Interrupted swap: the desired build is live on the alternate
    // port. Complete the swap — stop it, start it on canonical.
    if (current?.live && current.hash === hash && proxy.strategy === 'swap' && current.port !== proxy.canonicalPort) {
      if (dryRun) {
        result.action = 'would-swap-home';
        result.port = proxy.canonicalPort;
        log(JSON.stringify(result));
        return result;
      }
      await stopInstance(current.pid, cfg.drainSeconds, cfg, current.unit);
      const fin = await spawnInstance(cfg, proxy, env, proxy.canonicalPort);
      const okFin = await waitHealthy(proxy, proxy.canonicalPort, cfg.healthTimeoutSeconds, fin.isDead);
      if (!okFin) {
        result.action = 'swap-failed';
        log(JSON.stringify(result));
        return result;
      }
      const home = { port: proxy.canonicalPort, hash, pid: fin.pid, unit: fin.unit, startedAt: new Date().toISOString(), validatedOnPort: current.port };
      writeJsonAtomic(stateFile(cfg, proxy.name), { current: home });
      writeEndpoint(cfg, proxy, { ...home, previousPort: current.port });
      result.action = 'rolled';
      result.port = home.port;
      result.pid = home.pid;
      log(JSON.stringify(result));
      return result;
    }

    // Foreign instance discovery/adoption: healthy, on a known port,
    // not our recorded current.
    const foreignPort = ports.find((p) => health[p].ok && !(current?.live && current.port === p));
    if (!current?.live && foreignPort) {
      const pid = endpoint && endpoint.port === foreignPort ? endpoint.pid : null;
      const started = pid ? pidStartedAt(pid) : null;
      const newestMtime = Math.max(0, ...proxy.hashFiles.map((f) => {
        try { return fs.statSync(f).mtimeMs; } catch { return 0; }
      }));
      const adoptable = started !== null && newestMtime <= started + 2000;
      if (adoptable) {
        const adopted = { port: foreignPort, hash, pid, startedAt: new Date(started).toISOString(), adopted: true };
        result.action = 'adopt';
        result.port = foreignPort;
        result.pid = pid;
        if (!dryRun) {
          writeJsonAtomic(stateFile(cfg, proxy.name), { current: adopted });
          writeEndpoint(cfg, proxy, { ...adopted, previousPort: null });
        }
        log(JSON.stringify(result));
        return result;
      }
      // Stale foreign instance: remember it as the instance to replace.
      current = { port: foreignPort, pid, hash: null, live: true, foreign: true };
    }

    if (dryRun) {
      result.action = current?.live ? 'would-roll' : 'would-start';
      result.port = current?.live ? otherPort(proxy, current.port) : proxy.canonicalPort;
      log(JSON.stringify(result));
      return result;
    }

    // ---- start a new instance ----
    // Target: 'ab' rolls to the other port; 'swap' validates on the
    // alternate; 'inplace' restarts where it is.
    let targetPort;
    if (proxy.strategy === 'inplace') targetPort = proxy.canonicalPort;
    // 'swap' validates on the alternate only when replacing a live
    // instance; a cold start goes straight to canonical.
    else if (proxy.strategy === 'swap') targetPort = current?.live ? altPort : proxy.canonicalPort;
    else targetPort = current?.live ? otherPort(proxy, current.port) : proxy.canonicalPort;

    if (proxy.strategy === 'inplace' && current?.live) {
      await stopInstance(current.pid, cfg.drainSeconds, cfg, current.unit);
      current = null;
      // The health snapshot predates the stop; re-probe or the
      // occupied check below aborts on the ghost of the instance we
      // just stopped (observed live on the sandbox relays).
      health[targetPort] = await checkHealth(proxy, targetPort, 1500);
    }
    // A leftover occupant on the target port that is our recorded
    // previous instance gets stopped; anything else aborts the roll.
    if (health[targetPort]?.ok) {
      if (state.previous && state.previous.port === targetPort && pidAlive(state.previous.pid)) {
        await stopInstance(state.previous.pid, cfg.drainSeconds, cfg, state.previous.unit);
      } else if (current?.port === targetPort && current.live) {
        // swap strategy validating while current already sits on alt:
        // fall through — spawning will fail to bind; treat as roll to canonical below.
      } else {
        result.action = 'abort-occupied';
        log(JSON.stringify(result));
        return result;
      }
    }

    const previousPort = current?.live ? current.port : null;
    const inst = await spawnInstance(cfg, proxy, env, targetPort);
    const newPid = inst.pid;
    const healthy = newPid !== null && (await waitHealthy(proxy, targetPort, cfg.healthTimeoutSeconds, inst.isDead));
    if (!healthy) {
      if (pidAlive(newPid)) await stopInstance(newPid, 2, cfg, inst.unit);
      else if (inst.unit) systemctl(cfg, ['stop', inst.unit]);
      result.action = 'roll-failed-old-kept';
      result.port = previousPort;
      if (inst.error) result.error = inst.error;
      writeJsonAtomic(stateFile(cfg, proxy.name), { ...state, lastError: { at: new Date().toISOString(), hash, targetPort } });
      log(JSON.stringify(result));
      return result;
    }

    const startedAt = new Date().toISOString();
    const newCurrent = { port: targetPort, hash, pid: newPid, unit: inst.unit, startedAt };

    if (proxy.strategy === 'swap' && targetPort !== proxy.canonicalPort) {
      // Candidate validated on the alternate port. Stop it and the old
      // instance, then start the same build on the canonical port.
      await stopInstance(newPid, cfg.drainSeconds, cfg, inst.unit);
      if (current?.live && current.pid !== newPid) await stopInstance(current.pid, cfg.drainSeconds, cfg, current.unit);
      const final = await spawnInstance(cfg, proxy, env, proxy.canonicalPort);
      const finalHealthy = final.pid !== null && (await waitHealthy(proxy, proxy.canonicalPort, cfg.healthTimeoutSeconds, final.isDead));
      if (!finalHealthy) {
        if (pidAlive(final.pid)) await stopInstance(final.pid, 2, cfg, final.unit);
        else if (final.unit) systemctl(cfg, ['stop', final.unit]);
        result.action = 'swap-failed';
        writeJsonAtomic(stateFile(cfg, proxy.name), { ...state, lastError: { at: new Date().toISOString(), hash, targetPort: proxy.canonicalPort } });
        log(JSON.stringify(result));
        return result;
      }
      newCurrent.port = proxy.canonicalPort;
      newCurrent.pid = final.pid;
      newCurrent.unit = final.unit;
      newCurrent.validatedOnPort = targetPort;
    } else {
      // Cut over first (endpoint flip), then drain the old instance.
      writeJsonAtomic(stateFile(cfg, proxy.name), { current: newCurrent, previous: current?.live ? current : null });
      writeEndpoint(cfg, proxy, { ...newCurrent, previousPort });
      if (proxy.wireCommand) {
        try {
          spawnSync(proxy.wireCommand[0], proxy.wireCommand.slice(1), {
            env: { ...process.env, PROXY_NAME: proxy.name, PROXY_PORT: String(targetPort), PROXY_PREVIOUS_PORT: String(previousPort ?? '') },
            stdio: 'inherit',
            timeout: 30000,
          });
        } catch (e) {
          result.wireError = String(e).slice(0, 120);
        }
      }
      if (current?.live && current.pid !== newPid) {
        await stopInstance(current.pid, cfg.drainSeconds, cfg, current.unit);
        // The fleet proxies unlink endpoint.json on graceful exit; the
        // old instance's shutdown therefore deletes the record the new
        // instance (and this cutover) just published. Rewrite it now
        // that the old instance is gone.
        writeEndpoint(cfg, proxy, { ...newCurrent, previousPort });
      }
      writeJsonAtomic(stateFile(cfg, proxy.name), { current: newCurrent });
    }

    if (proxy.strategy === 'swap') {
      writeJsonAtomic(stateFile(cfg, proxy.name), { current: newCurrent });
      writeEndpoint(cfg, proxy, { ...newCurrent, previousPort });
    }

    result.action = current?.live || previousPort !== null ? 'rolled' : 'started';
    result.port = newCurrent.port;
    result.pid = newCurrent.pid;
    log(JSON.stringify(result));
    return result;
  } finally {
    if (lock) { try { fs.unlinkSync(lock); } catch { /* already gone */ } }
  }
};

// ---------- main ----------

const main = async () => {
  const args = process.argv.slice(2);
  const get = (flag) => {
    const i = args.indexOf(flag);
    return i >= 0 ? args[i + 1] : null;
  };
  const configPath = get('--config');
  if (!configPath) {
    console.error('usage: proxy-supervise.mjs --config <config.json> [--only <name>] [--status] [--dry-run]');
    process.exit(2);
  }
  const cfg = loadConfig(configPath);
  const only = get('--only');
  const selected = cfg.proxies.filter((p) => !only || p.name === only);
  if (selected.length === 0) {
    console.error(`no proxy named ${only} in ${configPath}`);
    process.exit(2);
  }
  if (args.includes('--status')) {
    for (const p of selected) {
      const st = readJson(stateFile(cfg, p.name));
      const ep = readJson(endpointFile(cfg, p.name));
      const h = ep ? await checkHealth(p, ep.port, 1200) : { ok: false };
      console.log(JSON.stringify({ name: p.name, state: st?.current ?? null, endpoint: ep, healthyNow: h.ok }));
    }
    return;
  }
  const dryRun = args.includes('--dry-run');
  let failures = 0;
  for (const p of selected) {
    try {
      const r = await reconcile(cfg, p, { dryRun });
      if (['roll-failed-old-kept', 'swap-failed', 'abort-occupied'].includes(r.action)) failures++;
    } catch (e) {
      failures++;
      console.log(JSON.stringify({ name: p.name, action: 'error', error: String(e).slice(0, 300) }));
    }
  }
  process.exit(failures > 0 ? 1 : 0);
};

if (import.meta.url === `file://${process.argv[1]}`) {
  main();
}
