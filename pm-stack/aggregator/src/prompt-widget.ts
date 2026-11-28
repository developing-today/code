// The prompt widget (#10032, design #9539 S1): a small form, a textarea with
// a target selector and submit, posting to /api/prompt. Pure markup; the
// caller decides where it sits on a page.
import { PROMPT_MODES, PROMPT_TARGETS, type PromptScope } from "./prompts.ts";

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
}

export interface PromptWidgetOptions {
  scope: PromptScope;
  page: string;
  sectionRef?: string | null;
}

export function renderPromptWidget(opts: PromptWidgetOptions): string {
  const section = opts.sectionRef ? ` data-section-ref="${escapeHtml(opts.sectionRef)}"` : "";
  const targets = PROMPT_TARGETS.map((t) => `<option value="${t}">${escapeHtml(t)}</option>`).join("");
  const modes = PROMPT_MODES.map((m) => `<option value="${m}">${escapeHtml(m)}</option>`).join("");
  return (
    `<form class="prompt-widget" method="post" action="/api/prompt" enctype="multipart/form-data"` +
    ` data-scope="${opts.scope}" data-page="${escapeHtml(opts.page)}"${section}>` +
    `<input type="hidden" name="scope" value="${opts.scope}">` +
    `<input type="hidden" name="page" value="${escapeHtml(opts.page)}">` +
    (opts.sectionRef ? `<input type="hidden" name="sectionRef" value="${escapeHtml(opts.sectionRef)}">` : "") +
    `<textarea name="prompt" rows="2" required aria-label="Prompt"></textarea>` +
    `<select name="target" aria-label="Target">${targets}</select>` +
    `<select name="mode" aria-label="Mode">${modes}</select>` +
    // #10036 S7: attachments ride the same submission as file parts;
    // the route persists them and the record carries storage refs.
    `<input type="file" name="attachments" multiple aria-label="Attachments">` +
    `<button type="submit">Send</button>` +
    `</form>`
  );
}
