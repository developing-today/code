// Operation forwarder — DSP-01 (developing-today/code#9445).
//
// The projector is bounded transport/projection glue: its HMAC-verified
// webhook intake (src/webhook.ts) proves a delivery really came from
// GitHub, and its pmp_deliveries rows dedupe *projection* application.
// Neither is canonical command acceptance. When a signed event carries
// an actionable Fleet command (the /fleet grammar of #9203), this
// module builds the operation envelope and hands it to the
// authoritative Fleet operation interface through an injected sink.
//
// Two separations are load-bearing:
//   * The idempotency key is semantic — repo + issue + comment id +
//     verb — never the GitHub delivery GUID. A redelivery, or a second
//     delivery of the same comment, therefore maps to the same command
//     and the Fleet side returns the stored receipt without a second
//     effect.
//   * The projector stores no acceptance state. The sink owns
//     authorization (authenticated principal, per-verb/resource) and
//     durable acceptance; this module only forwards envelopes and
//     surfaces receipts. No sink is configured in the default serve()
//     wiring yet, so forwarding stays inert until the Fleet authority
//     adapter is attached — no second ingress, no new service.

/** Canonical verb tokens of the /fleet command grammar (#9203 §3.1). */
export type OperationVerb =
  | "status"
  | "triage"
  | "start"
  | "resume"
  | "retry"
  | "park"
  | "unblock"
  | "priority"
  | "do-now"
  | "merge"
  | "agent-status";

const SINGLE_TOKEN_VERBS: ReadonlySet<string> = new Set([
  "status",
  "triage",
  "start",
  "resume",
  "retry",
  "park",
  "unblock",
  "priority",
  "do-now",
  "merge",
]);

/**
 * Parse a comment body as a /fleet command. Only a leading `/fleet`
 * token counts — a command quoted mid-comment is discussion, not a
 * command. Returns the canonical verb and the remaining argument text,
 * or null when the body carries no command.
 */
export function parseFleetCommand(body: string): { verb: OperationVerb; args: string } | null {
  const text = body.trim();
  const head = /^\/fleet(?:\s+|$)/.exec(text);
  if (!head) return null;
  const rest = text.slice(head[0].length).trim();
  if (!rest) return null;
  const lower = rest.toLowerCase();
  if (lower === "agent status" || lower.startsWith("agent status ")) {
    return { verb: "agent-status", args: rest.slice("agent status".length).trim() };
  }
  const token = lower.split(/\s+/, 1)[0];
  if (SINGLE_TOKEN_VERBS.has(token)) {
    return { verb: token as OperationVerb, args: rest.slice(token.length).trim() };
  }
  return null;
}

/** A signed-source event envelope handed to the Fleet authority. */
export interface OperationEnvelope {
  /** Semantic command identity; never derived from the delivery GUID. */
  idempotencyKey: string;
  /** Provenance only: the GitHub delivery that carried this attempt. */
  deliveryId: string;
  event: string;
  /** Authenticated actor from the verified provider payload. */
  actor: string;
  verb: OperationVerb;
  args: string;
  /** "owner/repo" the command acts on. */
  repo: string;
  /** Issue/PR number the command acts on. */
  issue: number;
  /** GitHub comment id the command was read from. */
  commentId: number;
}

/** The Fleet authority's stored acceptance receipt for one command. */
export interface OperationReceipt {
  receiptId: string;
  idempotencyKey: string;
  /** True when this receipt was already stored by an earlier forward. */
  duplicate: boolean;
}

/**
 * The authoritative Fleet operation interface, as a sink the server
 * injects. Implementations authorize the envelope's actor per
 * verb/resource and record durable acceptance keyed by
 * `envelope.idempotencyKey`, returning the stored receipt on repeats.
 * A rejected command or an unavailable authority is an error throw,
 * never a fabricated receipt.
 */
export interface FleetOperationSink {
  accept(envelope: OperationEnvelope): Promise<OperationReceipt>;
}

/**
 * Build the operation envelope for a verified webhook payload, or
 * null when the event carries no command. Only issue_comment events
 * are actionable in this slice: the command text, actor, and resource
 * all come from the signed payload itself.
 */
export function operationEnvelopeFor(input: {
  deliveryId: string;
  event: string;
  payload: Record<string, any>;
}): OperationEnvelope | null {
  const { deliveryId, event, payload } = input;
  if (event !== "issue_comment") return null;
  if (payload?.action !== "created" && payload?.action !== "edited") return null;
  const comment = payload?.comment;
  const repo = payload?.repository?.full_name;
  const issue = payload?.issue?.number;
  const actor = comment?.user?.login ?? payload?.sender?.login;
  if (typeof comment?.body !== "string") return null;
  if (typeof repo !== "string" || typeof issue !== "number") return null;
  if (typeof comment?.id !== "number" || typeof actor !== "string") return null;
  const command = parseFleetCommand(comment.body);
  if (!command) return null;
  return {
    idempotencyKey: `ghcmd:v1:${repo}#${issue}:comment:${comment.id}:${command.verb}`,
    deliveryId,
    event,
    actor,
    verb: command.verb,
    args: command.args,
    repo,
    issue,
    commentId: comment.id,
  };
}

/**
 * Thin forwarding seam: the server calls this on every verified
 * delivery that carries a command — including redeliveries whose
 * projection is already applied — and the sink's semantic dedup, not
 * the projector's delivery table, decides whether an effect repeats.
 */
export class OperationForwarder {
  constructor(private readonly sink: FleetOperationSink) {}

  async forward(envelope: OperationEnvelope): Promise<OperationReceipt> {
    return this.sink.accept(envelope);
  }
}
