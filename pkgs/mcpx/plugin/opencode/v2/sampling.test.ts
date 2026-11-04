/**
 * Sampling answers (#1858 item (4), #2841), against stubs: a fake
 * broker client serving canned rows and recording answers, and a
 * fake ctx.generate.text. The round-trip asserted here is the one
 * the daemon performs for real: a sample-mode broker row's
 * requestedSchema is the CreateMessageRequest params, and the
 * answer posted back must be a CreateMessageResult the broker's
 * ValidateSampleResult accepts (role "assistant", non-empty model,
 * text content, a stop reason).
 *
 *   bun test plugin/opencode/v2/sampling.test.ts
 */
import { describe, expect, test } from "bun:test";
import {
  createMessageResult,
  createSamplingAnswerer,
  samplingModel,
  samplingPrompt,
  type BrokerRow,
  type SamplingDaemonLike,
} from "./sampling.ts";

const sampleRow = (over: Partial<BrokerRow> = {}): BrokerRow => ({
  id: "elc-1",
  mode: "sample",
  audience: "agent",
  requestedSchema: {
    messages: [{ role: "user", content: { type: "text", text: "Say hello." } }],
    systemPrompt: "Be brief.",
    modelPreferences: { hints: [{ name: "hinted-model" }] },
    maxTokens: 64,
  },
  ...over,
});

const stubClient = (
  rows: BrokerRow[],
): {
  client: SamplingDaemonLike;
  answers: Array<{ id: string; action: string; content?: unknown }>;
} => {
  const answers: Array<{ id: string; action: string; content?: unknown }> = [];
  const client: SamplingDaemonLike = {
    pendingElicitations: async () => rows as unknown[],
    answer: async (id, action, content) => {
      answers.push({ id, action, content });
      return {};
    },
  } as SamplingDaemonLike;
  return { client, answers };
};

describe("samplingPrompt", () => {
  test("flattens system prompt and message text in order", () => {
    const prompt = samplingPrompt({
      systemPrompt: "Be brief.",
      messages: [
        { role: "user", content: { type: "text", text: "Say hello." } },
        {
          role: "assistant",
          content: [
            { type: "text", text: "Hello." },
            { type: "image", data: "..." },
          ],
        },
        { role: "user", content: "Again, please." },
      ],
    });
    expect(prompt).toBe(
      "Be brief.\n\nuser: Say hello.\n\nassistant: Hello.\n[image content omitted]\n\nuser: Again, please.",
    );
  });

  test("refuses a request carrying tools or a toolChoice", () => {
    const base = { messages: [{ role: "user", content: { type: "text", text: "hi" } }] };
    expect(samplingPrompt({ ...base, tools: [{ name: "t" }] })).toBeUndefined();
    expect(samplingPrompt({ ...base, toolChoice: "auto" })).toBeUndefined();
  });

  test("refuses a request with no text at all", () => {
    expect(samplingPrompt({ messages: [] })).toBeUndefined();
    expect(samplingPrompt({})).toBeUndefined();
  });
});

describe("samplingModel", () => {
  test("prefers the first non-empty hint, else the generate label", () => {
    expect(samplingModel({ modelPreferences: { hints: [{ name: "" }, { name: "m2" }] } })).toBe("m2");
    expect(samplingModel({})).toBe("opencode-v2-generate");
  });
});

describe("createMessageResult", () => {
  test("is the CreateMessageResult shape the broker validates", () => {
    expect(createMessageResult("Hello.", "m")).toEqual({
      role: "assistant",
      model: "m",
      content: { type: "text", text: "Hello." },
      stopReason: "endTurn",
    });
  });
});

describe("the sampling round-trip", () => {
  test("a sample row is generated and answered as a CreateMessageResult", async () => {
    const { client, answers } = stubClient([sampleRow()]);
    const prompts: string[] = [];
    const answerer = createSamplingAnswerer({
      client: async () => client,
      generate: async ({ prompt }) => {
        prompts.push(prompt);
        return { text: "Hello." };
      },
    });
    expect(await answerer.pollOnce()).toBe(1);
    expect(prompts).toEqual(["Be brief.\n\nuser: Say hello."]);
    expect(answers).toEqual([
      {
        id: "elc-1",
        action: "accept",
        content: {
          role: "assistant",
          model: "hinted-model",
          content: { type: "text", text: "Hello." },
          stopReason: "endTurn",
        },
      },
    ]);
  });

  test("the { data: { text } } generate envelope is unwrapped", async () => {
    const { client, answers } = stubClient([sampleRow()]);
    const answerer = createSamplingAnswerer({
      client: async () => client,
      generate: async () => ({ data: { text: "Wrapped." } }),
    });
    expect(await answerer.pollOnce()).toBe(1);
    expect((answers[0]?.content as { content: { text: string } }).content.text).toBe("Wrapped.");
  });

  test("form rows, human rows and tool-carrying samples are left open", async () => {
    const rows = [
      sampleRow({ id: "elc-form", mode: "form" }),
      sampleRow({ id: "elc-human", audience: "human" }),
      sampleRow({
        id: "elc-tools",
        requestedSchema: {
          messages: [{ role: "user", content: { type: "text", text: "hi" } }],
          tools: [{ name: "t", description: "d", inputSchema: { type: "object" } }],
        },
      }),
    ];
    const { client, answers } = stubClient(rows);
    let generated = 0;
    const answerer = createSamplingAnswerer({
      client: async () => client,
      generate: async () => {
        generated++;
        return { text: "x" };
      },
    });
    expect(await answerer.pollOnce()).toBe(0);
    expect(generated).toBe(0);
    expect(answers).toEqual([]);
  });

  test("a failed generation leaves the row open for the next poll", async () => {
    const { client, answers } = stubClient([sampleRow()]);
    let calls = 0;
    const answerer = createSamplingAnswerer({
      client: async () => client,
      generate: async () => {
        calls++;
        if (calls === 1) throw new Error("model unavailable");
        return { text: "Second try." };
      },
    });
    expect(await answerer.pollOnce()).toBe(0);
    expect(answers).toEqual([]);
    expect(await answerer.pollOnce()).toBe(1);
    expect(answers.length).toBe(1);
  });

  test("no daemon means no answers and no throw", async () => {
    const answerer = createSamplingAnswerer({
      client: async () => undefined,
      generate: async () => ({ text: "x" }),
    });
    expect(await answerer.pollOnce()).toBe(0);
  });
});
