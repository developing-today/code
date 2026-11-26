// #10038 S2 (design #9539 §3.3): pool tiers, effort override, notify toggle.
import { describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PromptsStore, validateNewPrompt } from "./prompts.ts";
import { renderPromptWidget } from "./prompt-widget.ts";
import { submissionFieldsFromForm } from "./prompt-attachments.ts";

const base = { scope: "page", page: "queue", sectionRef: null, mode: "async", prompt: "hello" } as const;

describe("pool tiers", () => {
  test("every pool tier is a target", () => {
    for (const target of ["pool:auto", "pool:low", "pool:medium", "pool:high"]) {
      expect(validateNewPrompt({ ...base, target }).ok).toBe(true);
    }
    expect(validateNewPrompt({ ...base, target: "pool:max" }).ok).toBe(false);
  });
});

describe("effort override", () => {
  test("a set effort is kept, auto leaves it out", () => {
    const set = validateNewPrompt({ ...base, target: "pool:medium", effort: "high" });
    expect(set.ok && set.value.effort).toBe("high");
    for (const effort of [undefined, null, ""]) {
      const r = validateNewPrompt({ ...base, target: "pool:auto", effort });
      expect(r.ok).toBe(true);
      if (r.ok) expect(r.value.effort).toBeUndefined();
    }
  });

  test("an unknown effort is refused", () => {
    expect(validateNewPrompt({ ...base, target: "pool:low", effort: "max" }).ok).toBe(false);
  });

  test("the store keeps the effort across a reload", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-prompt-effort-"));
    try {
      const path = join(dir, "prompts.json");
      const r = validateNewPrompt({ ...base, target: "pool:high", effort: "low" });
      if (!r.ok) throw new Error(r.error);
      const rec = await new PromptsStore(path).add(r.value);
      expect(rec.effort).toBe("low");
      const reloaded = new PromptsStore(path);
      expect((await reloaded.get(rec.id))?.effort).toBe("low");
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});

describe("widget", () => {
  test("offers the pool tiers, an auto-first effort selector and the notify toggle", () => {
    const html = renderPromptWidget({ scope: "page", page: "queue" });
    for (const t of ["pool:low", "pool:medium", "pool:high"]) expect(html).toContain(`<option value="${t}">`);
    expect(html).toContain('<select name="effort" aria-label="Effort"><option value="">auto effort</option>');
    expect(html).toContain('<input type="checkbox" name="notify" value="true">');
  });

  test("the form path carries effort and the ticked notify box", () => {
    const form = new FormData();
    for (const [k, v] of Object.entries({
      scope: "page",
      page: "queue",
      target: "pool:low",
      mode: "async",
      effort: "medium",
      notify: "true",
      prompt: "hello",
    }))
      form.set(k, v);
    const r = validateNewPrompt(submissionFieldsFromForm(form));
    expect(r.ok).toBe(true);
    if (r.ok) expect([r.value.target, r.value.effort, r.value.notify]).toEqual(["pool:low", "medium", true]);
  });
});
