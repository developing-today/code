/**
 * Sampling answers for the v2 plugin (#1858 SET-REMAINDER item (4),
 * #2841): mcpx relays a server's sampling/createMessage to its broker
 * as a sample-mode question routed to the agent audience ("sampling
 * asks a model, and the agent driving mcpx has one" — elicit.Route).
 * Until this module, the v2 plugin never looked at those rows, so
 * every sampling request sat in the broker until its 120s TTL expired
 * and the upstream heard an internal error.
 *
 * The v2 surface's answer is ctx.generate.text: one stateless
 * completion taking a plain prompt string and returning { text }
 * (pinned v2 source, protocol generate group: POST
 * /api/experimental/generate, { prompt, model? } -> { data: { text } };
 * docs/compare/register/plugin-apis.md PLG-33). It has no messages,
 * system prompt or tool use, so the CreateMessageRequest params the
 * broker stores in the row's requestedSchema are flattened to a
 * prompt: system prompt first, then each message's text blocks in
 * order, labelled by role. Non-text blocks are named, not dropped
 * silently. A request that carries tools or a toolChoice cannot be
 * honoured by a plain-text completion, so it is left open for another
 * answerer rather than answered with a result that pretends the
 * tools were available.
 *
 * The answer posted back is a CreateMessageResult — the shape the
 * broker validates (elicit.ValidateSampleResult): role "assistant",
 * a non-empty model, one text content block, stopReason "endTurn".
 * The model names the request's first model-preference hint when it
 * gave one (hints are expressly ignorable and unchecked upstream);
 * generate itself is called without a model, so the host's default
 * model does the work.
 *
 * Trigger: there is no push surface for broker rows in the v2 plugin
 * context, so setup starts a watcher that polls the daemon's elicit
 * list every 5s — comfortably inside the 120s sample TTL — and stops
 * on the setup cleanup. Every failure is swallowed: a missing daemon,
 * a failed generation or a lost answer race (the broker refuses a
 * second answer) leaves the question open for the next poll or
 * another answerer, and never fails the session or setup.
 */
import type { DaemonClient } from "../../core/daemon.ts";

/** The slice of the daemon client the watcher uses. */
export type SamplingDaemonLike = Pick<DaemonClient, "pendingElicitations" | "answer">;

/** ctx.generate.text, structurally: prompt in, text out. */
export type GenerateTextLike = (input: { prompt: string; model?: string }) => Promise<unknown>;

/** One broker row from GET /v1/elicit, the fields sampling reads. */
export type BrokerRow = {
  id?: unknown;
  mode?: unknown;
  audience?: unknown;
  requestedSchema?: unknown;
};

type SamplingParams = {
  messages?: unknown;
  systemPrompt?: unknown;
  modelPreferences?: { hints?: Array<{ name?: unknown }> };
  tools?: unknown;
  toolChoice?: unknown;
};

const textOf = (content: unknown): string[] => {
  if (typeof content === "string") return [content];
  if (Array.isArray(content)) {
    const out: string[] = [];
    for (const block of content) out.push(...textOf(block));
    return out;
  }
  if (content !== null && typeof content === "object") {
    const b = content as { type?: unknown; text?: unknown };
    if (b.type === "text" && typeof b.text === "string") return [b.text];
    if (typeof b.type === "string") return [`[${b.type} content omitted]`];
  }
  return [];
};

/**
 * Flatten CreateMessageRequest params to the plain prompt
 * ctx.generate.text takes. Returns undefined when the request cannot
 * be answered by a plain-text completion: it carries tools or a
 * toolChoice, or it has no text content at all.
 */
export function samplingPrompt(params: SamplingParams): string | undefined {
  if (params.tools !== undefined || params.toolChoice !== undefined) return undefined;
  const parts: string[] = [];
  if (typeof params.systemPrompt === "string" && params.systemPrompt !== "") {
    parts.push(params.systemPrompt);
  }
  const messages = Array.isArray(params.messages) ? params.messages : [];
  for (const message of messages) {
    if (message === null || typeof message !== "object") continue;
    const m = message as { role?: unknown; content?: unknown };
    const texts = textOf(m.content);
    if (texts.length === 0) continue;
    const role = typeof m.role === "string" ? m.role : "user";
    parts.push(`${role}: ${texts.join("\n")}`);
  }
  if (parts.length === 0) return undefined;
  return parts.join("\n\n");
}

/** The model name a sampling answer reports: the first hint, else a label. */
export function samplingModel(params: SamplingParams): string {
  const hints = params.modelPreferences?.hints;
  if (Array.isArray(hints)) {
    for (const hint of hints) {
      if (typeof hint?.name === "string" && hint.name !== "") return hint.name;
    }
  }
  return "opencode-v2-generate";
}

/** The CreateMessageResult the broker validates and relays upstream. */
export function createMessageResult(text: string, model: string): Record<string, unknown> {
  return {
    role: "assistant",
    model,
    content: { type: "text", text },
    stopReason: "endTurn",
  };
}

const generateText = async (generate: GenerateTextLike, prompt: string): Promise<string> => {
  const out = (await generate({ prompt })) as { text?: unknown; data?: { text?: unknown } } | null | undefined;
  const text = typeof out?.text === "string" ? out.text : out?.data?.text;
  if (typeof text !== "string") throw new Error("generate.text returned no text");
  return text;
};

export type SamplingDeps = {
  client: () => Promise<SamplingDaemonLike | undefined>;
  generate: GenerateTextLike;
};

/**
 * One poll: answer every open sample-mode, agent-audience broker row
 * that a plain-text completion can honour. Returns how many answers
 * were posted. Rows already being answered by this watcher are
 * skipped, so a slow generation is never started twice for one row.
 */
export function createSamplingAnswerer(deps: SamplingDeps): {
  pollOnce: () => Promise<number>;
} {
  const inFlight = new Set<string>();
  const pollOnce = async (): Promise<number> => {
    const client = await deps.client().catch(() => undefined);
    if (!client) return 0;
    const rows = (await client.pendingElicitations().catch(() => [])) as BrokerRow[];
    let answered = 0;
    for (const row of rows) {
      if (row.mode !== "sample" || row.audience !== "agent") continue;
      if (typeof row.id !== "string" || row.id === "" || inFlight.has(row.id)) continue;
      const params = (row.requestedSchema ?? {}) as SamplingParams;
      const prompt = samplingPrompt(params);
      if (prompt === undefined) continue;
      inFlight.add(row.id);
      try {
        const text = await generateText(deps.generate, prompt);
        await client.answer(row.id, "accept", createMessageResult(text, samplingModel(params)));
        answered++;
      } catch {
        // Generation failed or the answer lost its race: the row
        // stays open for the next poll or another answerer.
      } finally {
        inFlight.delete(row.id);
      }
    }
    return answered;
  };
  return { pollOnce };
}

/**
 * The polling watcher setup starts. The interval is unref'd where
 * the runtime supports it so a forgotten stop can never keep a host
 * process alive; stop() is still the contract (setup's cleanup).
 */
export function startSamplingWatcher(deps: SamplingDeps, intervalMs = 5000): { stop: () => void } {
  const answerer = createSamplingAnswerer(deps);
  let stopped = false;
  const tick = (): void => {
    if (stopped) return;
    void answerer.pollOnce().catch(() => 0);
  };
  const timer = setInterval(tick, intervalMs) as unknown as {
    unref?: () => void;
  };
  timer.unref?.();
  tick();
  return {
    stop: () => {
      stopped = true;
      clearInterval(timer as unknown as ReturnType<typeof setInterval>);
    },
  };
}
