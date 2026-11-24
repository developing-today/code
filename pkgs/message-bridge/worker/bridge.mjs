#!/usr/bin/env node
// bridge — a tiny client for the message bridge's class/name notes.
//
//   bridge send <to> <note...>   send a note (to may use * in a segment)
//   bridge inbox                 list this identity's pending notes
//   bridge ack <id> [<id>...]    acknowledge notes by id
//
// The caller's token never appears in arguments or output. It comes
// from, in order:
//   BRIDGE_TOKEN            a token value in the environment
//   BRIDGE_TOKEN_SECRET     the name of a vault secret holding it
//   --token-secret <name>   the same, as a flag before the command
//   bridge.archer-token     Archer's default vault secret name
// Drew's sending token is in the vault as bridge.send-token, so Drew's
// shell uses BRIDGE_TOKEN_SECRET=bridge.send-token (or the flag).
// BRIDGE_URL overrides https://bridge.security.cab.

import { execFileSync } from "node:child_process";

const BASE = (process.env.BRIDGE_URL ?? "https://bridge.security.cab").replace(/\/+$/, "");

function usage() {
  console.error(
    [
      "usage:",
      "  bridge [--token-secret <vault-name>] send <to> <note...>",
      "  bridge [--token-secret <vault-name>] inbox",
      "  bridge [--token-secret <vault-name>] ack <id> [<id>...]",
    ].join("\n"),
  );
  process.exit(2);
}

const argv = process.argv.slice(2);
let tokenSecret = process.env.BRIDGE_TOKEN_SECRET ?? null;
while (argv[0] === "--token-secret") {
  argv.shift();
  tokenSecret = argv.shift();
  if (!tokenSecret) usage();
}
const [command, ...rest] = argv;

function token() {
  if (process.env.BRIDGE_TOKEN) return process.env.BRIDGE_TOKEN.trim();
  const name = tokenSecret ?? "bridge.archer-token";
  return execFileSync("fleet", ["secrets", "get", name], { encoding: "utf8" }).trim();
}

async function call(path, init = {}) {
  const res = await fetch(`${BASE}${path}`, {
    ...init,
    headers: { authorization: `Bearer ${token()}`, "content-type": "application/json", ...(init.headers ?? {}) },
  });
  const text = await res.text();
  let data;
  try {
    data = JSON.parse(text);
  } catch {
    data = { raw: text };
  }
  if (!res.ok) {
    console.error(`bridge: ${path} answered ${res.status}: ${data.error ?? text.trim()}`);
    process.exit(1);
  }
  return data;
}

if (command === "send") {
  const [to, ...words] = rest;
  const note = words.join(" ");
  if (!to || !note) usage();
  const out = await call("/notes/send", { method: "POST", body: JSON.stringify({ to, note }) });
  console.log(JSON.stringify(out, null, 2));
} else if (command === "inbox") {
  if (rest.length) usage();
  const out = await call("/notes/inbox");
  console.log(JSON.stringify(out, null, 2));
} else if (command === "ack") {
  const ids = rest.map((raw) => Number(raw));
  if (!ids.length || !ids.every((id) => Number.isInteger(id) && id > 0)) usage();
  const out = await call("/notes/ack", { method: "POST", body: JSON.stringify({ ids }) });
  console.log(JSON.stringify(out, null, 2));
} else {
  usage();
}
