// Any-to-any messaging: one address grammar for everyone who talks
// through the bridge, so a connector (Claude, ChatGPT) or a fleet agent
// can send a message or an instruction to any other.
//
// Addresses (case-insensitive; a list or a comma-separated string):
//
//   drew (or owner, central)   Drew: from a connector, an outbound row on
//                              its own channel (what send_message always
//                              did); from an agent, a post to central
//   claude, chatgpt            a connector's channel (also channel:<name>)
//   connectors                 every other connector
//   agent:<name>               one fleet agent (a bare name works too)
//   group:<name>               every registered agent in that group
//   agents                     every registered agent
//   all                        every other connector and every agent
//
// The sender is never its own recipient: "all" and "connectors" leave it
// out, and naming yourself is an error. kind is "message" (default) or
// "instruction" ("command" is the same thing; agents' mailboxes call it
// that). Drew only gets messages: an instruction is for an assistant or
// an agent to act on.
//
// Every row another assistant or agent sends records who sent it in
// from_addr ("channel:claude", "agent:muse-3"), so the recipient can tell
// it from something Drew sent and answer it with reply_to.

import { CHANNELS, CHANNEL_NAMES, InputError, MAX_BODY } from "./messages.js";

export const ADDRESS_HELP = "drew, claude, chatgpt, connectors, agent:<name>, group:<name>, agents or all";
const DREW = ["drew", "owner", "central"];
const NAME = /^[a-z0-9][a-z0-9._-]{0,63}$/;
const MAX_ADDRESSES = 200;

/** "message" or "instruction"; "command" is an instruction. */
export function relayKind(kind) {
  if (kind === undefined || kind === null || kind === "" || kind === "message") return "message";
  if (kind === "instruction" || kind === "command") return "instruction";
  throw new InputError("kind must be message or instruction");
}

/** The sender's own channel or agent name, from its address. */
function self(from) {
  const [type, name] = from.split(":");
  return { channel: type === "channel" ? name : null, agent: type === "agent" ? name : null };
}

/**
 * Parses `to` for a sender ("channel:<c>" or "agent:<n>"). Returns
 * { drew, channels: [..], fleet: [fleet targets for parseTargets], all }.
 * An empty `to` means Drew, as before addresses existed.
 */
export function parseAddresses(to, from) {
  const me = self(from);
  const out = { drew: false, channels: new Set(), fleet: [], all: false };
  if (to === undefined || to === null || to === "" || (Array.isArray(to) && !to.length)) {
    out.drew = true;
    return { ...out, channels: [] };
  }
  const list = Array.isArray(to) ? to : String(to).split(",");
  if (list.length > MAX_ADDRESSES) throw new InputError(`at most ${MAX_ADDRESSES} addresses`);
  const otherChannels = CHANNELS.filter((c) => c !== me.channel);
  for (const raw of list) {
    if (typeof raw !== "string") throw new InputError(`each address must be a string: ${ADDRESS_HELP}`);
    const t = raw.trim().toLowerCase();
    if (t === "") continue;
    if (DREW.includes(t)) {
      out.drew = true;
    } else if (t === "connectors") {
      for (const c of otherChannels) out.channels.add(c);
    } else if (t === "all" || t === "everyone") {
      for (const c of otherChannels) out.channels.add(c);
      out.fleet.push("all");
      out.all = true;
    } else if (t === "agents" || t === "group:all") {
      out.fleet.push("all");
    } else if (CHANNELS.includes(t) || (t.startsWith("channel:") && CHANNELS.includes(t.slice(8)))) {
      const c = t.startsWith("channel:") ? t.slice(8) : t;
      if (c === me.channel) throw new InputError(`${CHANNEL_NAMES[c]} can't send to itself`);
      out.channels.add(c);
    } else if (t.startsWith("channel:")) {
      throw new InputError(`no connector is called ${t.slice(8)}; connectors are ${CHANNELS.join(", ")}`);
    } else if (t.startsWith("group:")) {
      out.fleet.push(t);
    } else {
      const name = t.startsWith("agent:") ? t.slice(6) : t;
      if (!NAME.test(name)) throw new InputError(`unknown address ${JSON.stringify(raw)}; use ${ADDRESS_HELP}`);
      if (name === me.agent) throw new InputError("an agent can't send to itself");
      out.fleet.push(`agent:${name}`);
    }
  }
  if (!out.drew && !out.channels.size && !out.fleet.length) {
    throw new InputError(`to must name at least one address: ${ADDRESS_HELP}`);
  }
  return { ...out, channels: CHANNELS.filter((c) => out.channels.has(c)) };
}

/**
 * Sends one message from a connector or an agent to every address in
 * `to`. Returns { messages, fleet, unregistered }: the connector rows and
 * the fleet rows it stored, so the caller can wake their recipients.
 * Everything is validated before the first row is written; the fleet
 * send goes first because only it can fail on state (an empty group).
 */
export async function relay(store, { from, to, body, kind, sender }) {
  if (typeof body !== "string" || body.trim() === "") throw new InputError("body must be a non-empty string");
  if (body.length > MAX_BODY) throw new InputError(`body is longer than ${MAX_BODY} characters`);
  const k = relayKind(kind);
  const where = parseAddresses(to, from);
  if (where.drew && k === "instruction") {
    throw new InputError("Drew gets messages, not instructions: send kind message to drew");
  }
  const me = self(from);
  const source = me.channel ? "mcp" : "fleet";
  let fleet = [];
  let unregistered = [];
  if (where.fleet.length) {
    const out = await store.fleet("send", {
      body,
      to: where.fleet,
      kind: k === "instruction" ? "command" : "message",
      sender,
      from_addr: from,
      exclude: me.agent,
      lenient: where.all,
    });
    fleet = out.messages;
    unregistered = out.unregistered ?? [];
  }
  const messages = [];
  for (const channel of where.channels) {
    messages.push(await store.add({ body, sender, direction: "inbound", source, channel, kind: k, from_addr: from }));
  }
  if (where.drew) {
    if (me.channel) {
      messages.push(await store.add({ body, sender, direction: "outbound", source: "mcp", channel: me.channel }));
    } else {
      fleet.push((await store.fleet("post", { agent: me.agent, body })).message);
    }
  }
  return { messages, fleet, unregistered };
}

/** A from_addr as the address that answers it: null (Drew) stays "drew". */
export const replyAddress = (fromAddr) => fromAddr ?? "drew";
