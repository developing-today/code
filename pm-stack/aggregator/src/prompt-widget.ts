// The prompt widget (#10032, design #9539 S1): a small form, a textarea with
// a target selector and submit, posting to /api/prompt. Pure markup; the
// caller decides where it sits on a page.
//
// #10037 S5: a widget can be opened on a specialized variant, which
// pre-fills the textarea with the variant's template and preselects its
// default target and mode. Both selectors stay editable, so every default
// can be overridden before sending.
import {
  PROMPT_MODES,
  PROMPT_TARGETS,
  PROMPT_VARIANT_SPECS,
  isSpecializedVariant,
  type PromptScope,
  type PromptVariant,
  type SpecializedPromptVariant,
} from "./prompts.ts";

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
}

export interface PromptWidgetOptions {
  scope: PromptScope;
  page: string;
  sectionRef?: string | null;
  /** #10037 S5: open the widget on a specialized variant. */
  variant?: PromptVariant;
}

function options(values: readonly string[], selected: string | null): string {
  return values
    .map((v) => `<option value="${v}"${v === selected ? " selected" : ""}>${escapeHtml(v)}</option>`)
    .join("");
}

export function renderPromptWidget(opts: PromptWidgetOptions): string {
  const section = opts.sectionRef ? ` data-section-ref="${escapeHtml(opts.sectionRef)}"` : "";
  const spec = isSpecializedVariant(opts.variant) ? PROMPT_VARIANT_SPECS[opts.variant] : null;
  const targets = options(PROMPT_TARGETS, spec ? spec.target : null);
  const modes = options(PROMPT_MODES, spec ? spec.mode : null);
  const variantAttr = spec ? ` data-variant="${opts.variant}"` : "";
  const variantInput = spec ? `<input type="hidden" name="variant" value="${opts.variant}">` : "";
  const label = spec ? `${spec.label} prompt` : "Prompt";
  return (
    `<form class="prompt-widget" method="post" action="/api/prompt" enctype="multipart/form-data"` +
    ` data-scope="${opts.scope}" data-page="${escapeHtml(opts.page)}"${section}${variantAttr}>` +
    `<input type="hidden" name="scope" value="${opts.scope}">` +
    `<input type="hidden" name="page" value="${escapeHtml(opts.page)}">` +
    (opts.sectionRef ? `<input type="hidden" name="sectionRef" value="${escapeHtml(opts.sectionRef)}">` : "") +
    variantInput +
    `<textarea name="prompt" rows="2" required aria-label="${escapeHtml(label)}">` +
    (spec ? escapeHtml(spec.template) : "") +
    `</textarea>` +
    `<select name="target" aria-label="Target">${targets}</select>` +
    `<select name="mode" aria-label="Mode">${modes}</select>` +
    // #10036 S7: attachments ride the same submission as file parts;
    // the route persists them and the record carries storage refs.
    `<input type="file" name="attachments" multiple aria-label="Attachments">` +
    `<button type="submit">Send</button>` +
    `</form>`
  );
}

/** #10037 S5: one pre-filled widget per specialized variant, collapsed
 *  under a single disclosure so a page can offer all five without
 *  crowding its primary prompt widget. */
export function renderPromptVariantWidgets(opts: Omit<PromptWidgetOptions, "variant">): string {
  const variants = Object.keys(PROMPT_VARIANT_SPECS) as SpecializedPromptVariant[];
  const items = variants
    .map(
      (v) =>
        `<section class="prompt-variant" data-variant="${v}">` +
        `<h4>${escapeHtml(PROMPT_VARIANT_SPECS[v].label)}</h4>` +
        renderPromptWidget({ ...opts, variant: v }) +
        `</section>`,
    )
    .join("");
  return `<details class="prompt-variants"><summary>Specialized tasks</summary>${items}</details>`;
}
