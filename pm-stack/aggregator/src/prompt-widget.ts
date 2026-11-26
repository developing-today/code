// The prompt widget (#10032, design #9539 S1): a small form, a textarea with
// a target selector and submit, posting to /api/prompt. Pure markup; the
// caller decides where it sits on a page.
//
// #10037 S5: a widget can be opened on a specialized variant, which
// pre-fills the textarea with the variant's template and preselects its
// default target and mode. Both selectors stay editable, so every default
// can be overridden before sending.
import {
  PROMPT_EFFORTS,
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
    // #10038 S2: effort override (blank is auto) and the opt-in push on
    // completion, which the record's notify flag carries.
    `<select name="effort" aria-label="Effort"><option value="">auto effort</option>${options(PROMPT_EFFORTS, null)}</select>` +
    `<label class="prompt-notify"><input type="checkbox" name="notify" value="true"> Notify me when answered</label>` +
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

/** #10032 S1 (design #9539 §2): the primary pages that carry a page-scope
 *  widget. Ids are the page manifest's (src/pages.ts); "attention" is the
 *  Needs you page. */
export const PRIMARY_PROMPT_PAGES: readonly string[] = [
  "overview",
  "today",
  "attention",
  "program",
  "queue",
  "roadmap",
  "board",
];

/** #10032 S1: a section-scope widget for one discrete row, collapsed so a
 *  long list stays readable. The ref is opaque to the store. */
export function renderSectionPrompt(page: string, sectionRef: string): string {
  return (
    `<details class="prompt-section"><summary class="muted">Ask about this</summary>` +
    renderPromptWidget({ scope: "section", page, sectionRef }) +
    `</details>`
  );
}

export interface PromptDockOptions {
  /** The page id the stack and agent widgets report as their origin. */
  page: string;
  /** Add the page-scope widget and the specialized variants. Only the
   *  primary pages get them; sub-pages keep the stack and agent widgets. */
  pageScope: boolean;
}

/** #10032 S1: the prompt dock in the shared footer. Every page gets the
 *  stack and agent widgets (design §2 "Lives in the nav/footer on every
 *  page"); primary pages also get the page-scope widget and the S5
 *  variants. It sits in the layout, outside any refreshing fragment, so a
 *  half-typed prompt survives a live refresh. */
export function renderPromptDock(opts: PromptDockOptions): string {
  const pagePart = opts.pageScope
    ? `<section class="prompt-scope" data-prompt-scope="page"><h3>Ask about this page</h3>` +
      renderPromptWidget({ scope: "page", page: opts.page }) +
      renderPromptVariantWidgets({ scope: "page", page: opts.page }) +
      `</section>`
    : "";
  return (
    `<details class="prompt-dock"${opts.pageScope ? " open" : ""}><summary>Ask Claude</summary>` +
    pagePart +
    `<section class="prompt-scope" data-prompt-scope="stack"><h3>Ask about the PM stack</h3>` +
    renderPromptWidget({ scope: "stack", page: opts.page }) +
    `</section>` +
    `<section class="prompt-scope" data-prompt-scope="agent"><h3>Hand a task to an agent</h3>` +
    renderPromptWidget({ scope: "agent", page: opts.page }) +
    `</section>` +
    `</details>`
  );
}
