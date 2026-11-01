#!/usr/bin/env node
// Based on the OpenClaw-generated wrapper present 2026-10-31. Keep its
// argument, diagnostics and process-lifecycle behavior when updating it;
// only the adapter and writable state locations are managed locally.
import { appendFileSync, existsSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { StringDecoder } from "node:string_decoder";

const codexHome = path.join(process.env.HOME, ".openclaw/acpx/codex-home/");
const codexAuthPath = path.join(process.env.HOME, ".openclaw/acpx/codex-home/auth.json");
const codexApiKey = (process.env.CODEX_API_KEY || process.env.OPENAI_API_KEY || "").trim();
let shouldWriteCodexApiKeyAuth = false;
if (codexApiKey) {
  if (!existsSync(codexAuthPath)) {
    shouldWriteCodexApiKeyAuth = true;
  } else {
    try {
      const existingCodexAuth = JSON.parse(readFileSync(codexAuthPath, "utf8"));
      shouldWriteCodexApiKeyAuth =
        !existingCodexAuth ||
        typeof existingCodexAuth !== "object" ||
        typeof existingCodexAuth.OPENAI_API_KEY === "string";
    } catch {
      shouldWriteCodexApiKeyAuth = true;
    }
  }
}
if (shouldWriteCodexApiKeyAuth) {
  writeFileSync(
    codexAuthPath,
    JSON.stringify({
      OPENAI_API_KEY: codexApiKey,
      tokens: null,
      last_refresh: null,
    }) + "\n",
    { mode: 0o600 },
  );
}
const env = {
  ...process.env,
  CODEX_HOME: codexHome,
};
const stderrLogFileNamePrefix = "codex-acp-wrapper.stderr";
const stderrLogMaxChars = 256 * 1024;

const openClawWrapperArgs = new Set([
  "--openclaw-acpx-lease-id",
  "--openclaw-gateway-instance-id",
  "--openclaw-codex-config"
]);

function readOpenClawWrapperArg(args, name) {
  const index = args.indexOf(name);
  if (index < 0) {
    return undefined;
  }
  const value = args[index + 1];
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function readOpenClawWrapperArgs(args, name) {
  const values = [];
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] !== name) {
      continue;
    }
    const value = args[index + 1];
    if (typeof value === "string" && value.trim()) {
      values.push(value.trim());
    }
    index += 1;
  }
  return values;
}

function safeDiagnosticFilePart(value) {
  const sanitized = String(value || "").replace(/[^A-Za-z0-9._-]/g, "_").slice(0, 120);
  return sanitized || "pid-" + process.pid;
}

function resolveStderrLogPath(args) {
  if (!stderrLogFileNamePrefix) {
    return undefined;
  }
  const leaseId =
    readOpenClawWrapperArg(args, "--openclaw-acpx-lease-id") ||
    "pid-" + process.pid;
  const fileName = stderrLogFileNamePrefix + "." + safeDiagnosticFilePart(leaseId) + ".log";
  return path.join(process.env.HOME, ".openclaw/acpx", fileName);
}

const diagnosticRedactionRules = [{"source":"(authorization\\s*[:=]\\s*bearer\\s+)[^\\s'\"<>]+","flags":"gi","replacement":"$1[REDACTED]"},{"source":"((?:api[_-]?key|apiKey|access[_-]?token|refresh[_-]?token|client[_-]?secret|token|secret|password|passwd|credential)\\s*[:=]\\s*)[^\\s'\"<>]+","flags":"gi","replacement":"$1[REDACTED]"},{"source":"(\"(?:apiKey|token|secret|password|passwd|accessToken|refreshToken)\"\\s*:\\s*\")[^\"]+","flags":"g","replacement":"$1[REDACTED]"},{"source":"([\"']?(?:api[-_]?key|apiKey|access[-_]?token|accessToken|refresh[-_]?token|refreshToken|id[-_]?token|idToken|auth[-_]?token|authToken|client[-_]?secret|clientSecret|app[-_]?secret|appSecret|token|secret|password|passwd|credential)[\"']?\\s*[:=]\\s*[\"']?)[^\"',}\\s<>]+","flags":"gi","replacement":"$1[REDACTED]"},{"source":"([?&](?:access[-_]?token|auth[-_]?token|refresh[-_]?token|api[-_]?key|client[-_]?secret|token|key|secret|password|pass|passwd|auth|signature)=)[^&\\s'\"<>]+","flags":"gi","replacement":"$1[REDACTED]"},{"source":"(--(?:api[-_]?key|token|secret|password|passwd)\\s+)[^\\s'\"]+","flags":"gi","replacement":"$1[REDACTED]"},{"source":"-----BEGIN [A-Z ]*PRIVATE KEY-----[\\s\\S]+?-----END [A-Z ]*PRIVATE KEY-----","flags":"g","replacement":"[REDACTED_PRIVATE_KEY]"},{"source":"\\b(sk-[A-Za-z0-9_-]{8,})\\b","flags":"g","replacement":"[REDACTED_OPENAI_KEY]"},{"source":"\\b(gh[pousr]_[A-Za-z0-9_]{20,})\\b","flags":"g","replacement":"[REDACTED_GITHUB_TOKEN]"},{"source":"\\b(github_pat_[A-Za-z0-9_]{20,})\\b","flags":"g","replacement":"[REDACTED_GITHUB_TOKEN]"},{"source":"\\b(xox[baprs]-[A-Za-z0-9-]{10,})\\b","flags":"g","replacement":"[REDACTED_SLACK_TOKEN]"},{"source":"\\b(gsk_[A-Za-z0-9_-]{10,})\\b","flags":"g","replacement":"[REDACTED_API_KEY]"},{"source":"\\b(AIza[0-9A-Za-z\\-_]{20,})\\b","flags":"g","replacement":"[REDACTED_GOOGLE_KEY]"},{"source":"\\b(ya29\\.[0-9A-Za-z_\\-./+=]{10,})\\b","flags":"g","replacement":"[REDACTED_GOOGLE_TOKEN]"},{"source":"\\b(eyJ[A-Za-z0-9_-]{10,}\\.[A-Za-z0-9_-]{10,}\\.[A-Za-z0-9_-]{10,})\\b","flags":"g","replacement":"[REDACTED_JWT]"},{"source":"\\b(pplx-[A-Za-z0-9_-]{10,})\\b","flags":"g","replacement":"[REDACTED_API_KEY]"},{"source":"\\b(npm_[A-Za-z0-9]{10,})\\b","flags":"g","replacement":"[REDACTED_NPM_TOKEN]"},{"source":"\\b(LTAI[A-Za-z0-9]{10,})\\b","flags":"g","replacement":"[REDACTED_ACCESS_KEY]"},{"source":"\\b(hf_[A-Za-z0-9]{10,})\\b","flags":"g","replacement":"[REDACTED_API_KEY]"},{"source":"\\bbot(\\d{6,}:[A-Za-z0-9_-]{20,})\\b","flags":"g","replacement":"bot[REDACTED_TELEGRAM_TOKEN]"},{"source":"\\b(\\d{6,}:[A-Za-z0-9_-]{20,})\\b","flags":"g","replacement":"[REDACTED_TELEGRAM_TOKEN]"}].map((rule) => [
  new RegExp(rule.source, rule.flags),
  rule.replacement,
]);

function redactDiagnosticText(text) {
  let redacted = text;
  for (const [pattern, replacement] of diagnosticRedactionRules) {
    redacted = redacted.replace(pattern, replacement);
  }
  return redacted;
}

function tailUtf16Safe(text, maxChars) {
  let start = Math.max(0, text.length - maxChars);
  const startsInsideSurrogatePair =
    start > 0 &&
    start < text.length &&
    text.charCodeAt(start) >= 0xdc00 &&
    text.charCodeAt(start) <= 0xdfff &&
    text.charCodeAt(start - 1) >= 0xd800 &&
    text.charCodeAt(start - 1) <= 0xdbff;
  if (startsInsideSurrogatePair) {
    start += 1;
  }
  return text.slice(start);
}

let pendingStderrLogText = "";
// Pipe chunks can split a UTF-8 sequence. Preserve decoder state so diagnostic
// capture does not manufacture replacement characters between chunks.
const stderrDecoder = new StringDecoder("utf8");
const stderrPrivateKeyEndPattern = /-----END [A-Z ]*PRIVATE KEY-----/;

function hasUnclosedPrivateKeyBlock(text) {
  let lastBeginIndex = -1;
  for (const match of text.matchAll(/-----BEGIN [A-Z ]*PRIVATE KEY-----/g)) {
    lastBeginIndex = match.index ?? lastBeginIndex;
  }
  if (lastBeginIndex === -1) {
    return -1;
  }
  return stderrPrivateKeyEndPattern.test(text.slice(lastBeginIndex)) ? -1 : lastBeginIndex;
}

function writeRedactedStderrLog(text) {
  if (!stderrLogPath) {
    return;
  }
  if (!text) {
    return;
  }
  try {
    appendFileSync(stderrLogPath, redactDiagnosticText(text), "utf8");
    const current = readFileSync(stderrLogPath, "utf8");
    if (current.length > stderrLogMaxChars) {
      writeFileSync(stderrLogPath, tailUtf16Safe(current, stderrLogMaxChars), "utf8");
    }
  } catch {
    // Stderr capture is diagnostic-only; never break the ACP adapter.
  }
}

function redactIncompletePrivateKeyTail(text) {
  const unclosedPrivateKeyStart = hasUnclosedPrivateKeyBlock(text);
  if (unclosedPrivateKeyStart === -1) {
    return text;
  }
  return text.slice(0, unclosedPrivateKeyStart) + "[REDACTED_PRIVATE_KEY]";
}

function flushFinalizedStderrLogText() {
  const lastLineBreak = pendingStderrLogText.lastIndexOf("\n");
  if (lastLineBreak === -1) {
    if (pendingStderrLogText.length > stderrLogMaxChars) {
      pendingStderrLogText = tailUtf16Safe(pendingStderrLogText, stderrLogMaxChars);
    }
    return;
  }
  let flushEnd = lastLineBreak + 1;
  const unclosedPrivateKeyStart = hasUnclosedPrivateKeyBlock(
    pendingStderrLogText.slice(0, flushEnd),
  );
  if (unclosedPrivateKeyStart !== -1) {
    flushEnd = unclosedPrivateKeyStart;
  }
  if (flushEnd <= 0) {
    if (pendingStderrLogText.length > stderrLogMaxChars) {
      pendingStderrLogText = tailUtf16Safe(pendingStderrLogText, stderrLogMaxChars);
    }
    return;
  }
  const finalizedText = pendingStderrLogText.slice(0, flushEnd);
  pendingStderrLogText = pendingStderrLogText.slice(flushEnd);
  writeRedactedStderrLog(finalizedText);
}

function appendStderrLog(chunk) {
  const text = stderrDecoder.write(chunk);
  if (!text) {
    return;
  }
  pendingStderrLogText += text;
  flushFinalizedStderrLogText();
}

function finishStderrLog() {
  pendingStderrLogText += stderrDecoder.end();
  const text = redactIncompletePrivateKeyTail(pendingStderrLogText);
  pendingStderrLogText = "";
  writeRedactedStderrLog(text);
}

function stripOpenClawWrapperArgs(args) {
  const stripped = [];
  for (let index = 0; index < args.length; index += 1) {
    const value = args[index];
    if (openClawWrapperArgs.has(value)) {
      index += 1;
      continue;
    }
    stripped.push(value);
  }
  return stripped;
}

const rawConfiguredArgs = process.argv.slice(2);
function isCodexConfigObject(value) {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function mergeCodexConfig(base, override) {
  const merged = Object.assign(Object.create(null), base);
  for (const [key, value] of Object.entries(override)) {
    const existing = merged[key];
    merged[key] =
      isCodexConfigObject(existing) && isCodexConfigObject(value)
        ? mergeCodexConfig(existing, value)
        : value;
  }
  return merged;
}

const openClawCodexConfigs = readOpenClawWrapperArgs(
  rawConfiguredArgs,
  "--openclaw-codex-config",
);
if (openClawCodexConfigs.length > 0) {
  let existingCodexConfig = {};
  if (typeof env.CODEX_CONFIG === "string" && env.CODEX_CONFIG.trim()) {
    try {
      const parsedCodexConfig = JSON.parse(env.CODEX_CONFIG);
      if (!parsedCodexConfig || typeof parsedCodexConfig !== "object" || Array.isArray(parsedCodexConfig)) {
        throw new Error("CODEX_CONFIG must be a JSON object");
      }
      existingCodexConfig = parsedCodexConfig;
    } catch {
      console.error("[openclaw] CODEX_CONFIG must be a valid JSON object");
      process.exit(1);
    }
  }
  for (const openClawCodexConfig of openClawCodexConfigs) {
    try {
      const parsedOpenClawCodexConfig = JSON.parse(openClawCodexConfig);
      if (
        !parsedOpenClawCodexConfig ||
        typeof parsedOpenClawCodexConfig !== "object" ||
        Array.isArray(parsedOpenClawCodexConfig)
      ) {
        throw new Error("invalid OpenClaw Codex config");
      }
      existingCodexConfig = mergeCodexConfig(existingCodexConfig, parsedOpenClawCodexConfig);
    } catch {
      console.error("[openclaw] invalid generated Codex ACP startup config");
      process.exit(1);
    }
  }
  env.CODEX_CONFIG = JSON.stringify(existingCodexConfig);
}
const stderrLogPath = resolveStderrLogPath(rawConfiguredArgs);
if (stderrLogPath) {
  try {
    rmSync(stderrLogPath, { force: true });
  } catch {
    // Diagnostic cleanup must never prevent the adapter from starting.
  }
}

const configuredArgs = stripOpenClawWrapperArgs(rawConfiguredArgs);

function resolveNpmCliPath() {
  const candidate = path.resolve(
    path.dirname(process.execPath),
    "..",
    "lib",
    "node_modules",
    "npm",
    "bin",
    "npm-cli.js",
  );
  return existsSync(candidate) ? candidate : undefined;
}

const npmCliPath = resolveNpmCliPath();
const installedBinPath = path.join(process.env.HOME, ".openclaw/acpx/bin/codex-acp.mjs");
let defaultCommand;
let defaultArgs;
// Plugin capture/install directories are disposable: a durable wrapper can
// outlive the path it captured, so re-check the target before trusting it.
if (installedBinPath && existsSync(installedBinPath)) {
  defaultCommand = process.execPath;
  defaultArgs = [installedBinPath];
} else if (npmCliPath) {
  defaultCommand = process.execPath;
  defaultArgs = [npmCliPath, "exec", "--yes", "--package", "@agentclientprotocol/codex-acp@2.1.1", "--", "codex-acp"];
} else {
  defaultCommand = process.platform === "win32" ? "npx.cmd" : "npx";
  defaultArgs = ["--yes", "--package", "@agentclientprotocol/codex-acp@2.1.1", "--", "codex-acp"];
}
const command =
  configuredArgs[0] === "--openclaw-run-configured" ? configuredArgs[1] : defaultCommand;
const args =
  configuredArgs[0] === "--openclaw-run-configured"
    ? configuredArgs.slice(2)
    : [...defaultArgs, ...configuredArgs];

if (!command) {
  console.error("[openclaw] missing configured Codex ACP command");
  process.exit(1);
}

const child = spawn(command, args, {
  detached: process.platform !== "win32",
  env,
  stdio: ["inherit", "inherit", "pipe"],
  windowsHide: true,
});

child.stderr?.on("data", (chunk) => {
  appendStderrLog(chunk);
  process.stderr.write(chunk);
});

let forceKillTimer;
let orphanCleanupStarted = false;
let childExitCode = 1;

function killChildTree(signal, options = {}) {
  if (!child.pid || (!options.force && child.killed)) {
    return;
  }
  if (process.platform !== "win32") {
    try {
      // The adapter can spawn grandchildren; signaling the process group keeps
      // the generated wrapper from leaving an ACP tree behind.
      process.kill(-child.pid, signal);
      return;
    } catch {
      // Fall back to direct child signaling below.
    }
  }
  child.kill(signal);
}

for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.once(signal, () => {
    killChildTree(signal);
  });
}

const originalParentPid = process.ppid;
const parentWatcher =
  process.platform === "win32"
    ? undefined
    : setInterval(() => {
        // Orphan detection: parent PID changed means our original parent died.
        // The new parent could be PID 1 (init) on bare-metal hosts, OR a
        // systemd user-session manager, OR a container init, OR a session
        // leader — depending on environment. Previously this only triggered
        // on PPID == 1, which missed all systemd-managed deployments and
        // leaked codex-acp adapter trees on every gateway restart.
        if (process.ppid === originalParentPid) {
          return;
        }
        if (orphanCleanupStarted) {
          return;
        }
        orphanCleanupStarted = true;
        if (parentWatcher) {
          clearInterval(parentWatcher);
        }
        killChildTree("SIGTERM");
        // Keep the wrapper alive long enough for stubborn adapters to receive
        // a forced fallback signal after SIGTERM.
        forceKillTimer = setTimeout(() => {
          killChildTree("SIGKILL", { force: true });
          childExitCode = 1;
        }, 1_500);
      }, 1_000);
parentWatcher?.unref?.();

child.on("error", (error) => {
  console.error(`[openclaw] failed to launch Codex ACP wrapper: ${error.message}`);
  process.exit(1);
});

child.on("exit", (code, signal) => {
  if (parentWatcher) {
    clearInterval(parentWatcher);
  }
  if (orphanCleanupStarted) {
    return;
  }
  if (forceKillTimer) {
    clearTimeout(forceKillTimer);
  }
  if (code !== null) {
    childExitCode = code;
    return;
  }
  childExitCode = signal ? 1 : 0;
});

child.on("close", () => {
  finishStderrLog();
  process.exit(childExitCode);
});
