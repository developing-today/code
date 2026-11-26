// #10037 S5 (design #9539 §2): specialized task variants.
import { describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  PROMPT_VARIANTS,
  PROMPT_VARIANT_SPECS,
  PromptsStore,
  validateNewPrompt,
  type SpecializedPromptVariant,
} from "./prompts.ts";
import { renderPromptVariantWidgets, renderPromptWidget } from "./prompt-widget.ts";
import { submissionFieldsFromForm } from "./prompt-attachments.ts";

const base = { scope: "page", page: "queue", sectionRef: null } as const;
const specialized = PROMPT_VARIANTS.filter((v) => v !== "free") as SpecializedPromptVariant[];

describe("variant specs", () => {
  test("every specialized variant has a template and the design's defaults", () => {
    expect(specialized).toEqual(["reprioritize", "detail-gather", "compare-options", "integrate-insights", "report"]);
    expect(Object.keys(PROMPT_VARIANT_SPECS).sort()).toEqual([...specialized].sort());
    const defaults = Object.fromEntries(
      specialized.map((v) => [v, [PROMPT_VARIANT_SPECS[v].target, PROMPT_VARIANT_SPECS[v].mode]]),
    );
    expect(defaults).toEqual({
      reprioritize: ["pool:auto", "async"],
      "detail-gather": ["muse", "async"],
      "compare-options": ["claude", "sync-block"],
      "integrate-insights": ["claude", "async"],
      report: ["pool:auto", "async"],
    });
    for (const v of specialized) expect(PROMPT_VARIANT_SPECS[v].template.trim()).not.toBe("");
  });
});

describe("validateNewPrompt with a variant", () => {
  test("fills target and mode from the variant when left out", () => {
    for (const v of specialized) {
      const spec = PROMPT_VARIANT_SPECS[v];
      const r = validateNewPrompt({
        ...base,
        variant: v,
        prompt: `${spec.template}the board`,
      });
      expect(r.ok).toBe(true);
      if (r.ok) {
        expect(r.value.target).toBe(spec.target);
        expect(r.value.mode).toBe(spec.mode);
        expect(r.value.variant).toBe(v);
      }
    }
  });

  test("an explicit target or mode overrides the variant default", () => {
    const r = validateNewPrompt({
      ...base,
      variant: "compare-options",
      target: "muse",
      mode: "async",
      prompt: "Compare these options: A or B",
    });
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.value.target).toBe("muse");
      expect(r.value.mode).toBe("async");
    }
  });

  test("empty form selectors count as left out", () => {
    const r = validateNewPrompt({
      ...base,
      variant: "report",
      target: "",
      mode: "",
      prompt: "Generate a report of quota",
    });
    expect(r.ok).toBe(true);
    if (r.ok) expect([r.value.target, r.value.mode]).toEqual(["pool:auto", "async"]);
  });

  test("rejects an unknown variant and a bare template", () => {
    expect(
      validateNewPrompt({
        ...base,
        variant: "summarize",
        target: "claude",
        mode: "async",
        prompt: "x",
      }).ok,
    ).toBe(false);
    const bare = validateNewPrompt({
      ...base,
      variant: "reprioritize",
      prompt: "Reprioritize the following items:  ",
    });
    expect(bare.ok).toBe(false);
  });

  test("free needs an explicit target and mode, like a plain submission", () => {
    expect(validateNewPrompt({ ...base, variant: "free", prompt: "hello" }).ok).toBe(false);
    const r = validateNewPrompt({
      ...base,
      variant: "free",
      target: "claude",
      mode: "async",
      prompt: "hello",
    });
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.value.variant).toBe("free");
  });

  test("a plain submission carries no variant", () => {
    const r = validateNewPrompt({
      ...base,
      target: "claude",
      mode: "async",
      prompt: "hello",
    });
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.value.variant).toBeUndefined();
  });

  test("the multipart path passes the variant through", () => {
    const form = new FormData();
    for (const [k, v] of Object.entries({
      scope: "page",
      page: "queue",
      variant: "detail-gather",
      prompt: "Gather details on #1",
    }))
      form.set(k, v);
    const r = validateNewPrompt(submissionFieldsFromForm(form));
    expect(r.ok).toBe(true);
    if (r.ok) expect([r.value.variant, r.value.target]).toEqual(["detail-gather", "muse"]);
  });
});

describe("store", () => {
  test("records keep their variant across a reload", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-prompt-variants-"));
    try {
      const path = join(dir, "prompts.json");
      const store = new PromptsStore(path);
      const r = validateNewPrompt({
        ...base,
        variant: "integrate-insights",
        prompt: "Integrate new insights since last review: roadmap",
      });
      if (!r.ok) throw new Error(r.error);
      const rec = await store.add(r.value);
      expect(rec.variant).toBe("integrate-insights");
      const reloaded = new PromptsStore(path);
      await reloaded.load();
      expect((await reloaded.get(rec.id))?.variant).toBe("integrate-insights");
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});

describe("widget", () => {
  test("a variant widget pre-fills the template and preselects its defaults", () => {
    const html = renderPromptWidget({
      scope: "page",
      page: "board",
      variant: "compare-options",
    });
    expect(html).toContain('data-variant="compare-options"');
    expect(html).toContain('<input type="hidden" name="variant" value="compare-options">');
    expect(html).toContain(">Compare these options: </textarea>");
    expect(html).toContain('<option value="claude" selected>');
    expect(html).toContain('<option value="sync-block" selected>');
  });

  test("a plain widget has no variant and no preselection", () => {
    const html = renderPromptWidget({ scope: "page", page: "board" });
    expect(html).not.toContain("variant");
    expect(html).not.toContain(" selected");
    expect(html).toContain("></textarea>");
  });

  test("the variant set renders one widget per specialized variant", () => {
    const html = renderPromptVariantWidgets({ scope: "stack", page: "today" });
    for (const v of specialized) expect(html).toContain(`<section class="prompt-variant" data-variant="${v}">`);
    expect(html.match(/<form /g)?.length).toBe(specialized.length);
  });
});
