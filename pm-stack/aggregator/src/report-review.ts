import { compareReportBlock, type ReportRecord, type ReportBlockComparison } from "./report-records.ts";

export interface ReportBlockReview {
  periodId: string;
  blockId: string;
  storedRecord: ReportRecord;
  renderedRecord: ReportRecord;
  comparison: ReportBlockComparison;
}

/** Pair current report blocks with the newest distinct rendered version that
 * has the same stable block ID. The write endpoint recomputes this comparison. */
export function reportBlockReviews(records: ReportRecord[], current: ReportRecord[]): ReportBlockReview[] {
  const reviews: ReportBlockReview[] = [];
  for (const storedRecord of current) {
    for (const storedBlock of storedRecord.blocks) {
      const relatedRecords = records.filter((record) =>
        record.period.id === storedRecord.period.id && record.templateVersion === storedRecord.templateVersion);
      const resolvedAt = relatedRecords
        .flatMap((record) => record.blocks
          .filter((block) => block.id === storedBlock.id && block.resolution)
          .map((block) => block.resolution!.resolvedAt))
        .sort()
        .at(-1);
      const candidates = records
        .filter((record) => record.period.id === storedRecord.period.id && record.templateVersion === storedRecord.templateVersion &&
          record.id !== storedRecord.id && (!resolvedAt || record.generatedAt > resolvedAt ||
            record.blocks.some((block) => block.id === storedBlock.id && block.resolution)))
        .sort((a, b) => b.generatedAt.localeCompare(a.generatedAt) || b.id.localeCompare(a.id));
      // A whitespace-only version is not a conflict and must not hide an older
      // substantive alternative. Search until the newest substantive match.
      const renderedRecord = candidates.find((record) => {
        const block = record.blocks.find((candidate) => candidate.id === storedBlock.id);
        return block !== undefined && compareReportBlock(storedBlock.text, block.text).conflict;
      });
      const renderedBlock = renderedRecord?.blocks.find((block) => block.id === storedBlock.id);
      if (!renderedRecord || !renderedBlock) continue;
      const comparison = compareReportBlock(storedBlock.text, renderedBlock.text);
      reviews.push({ periodId: storedRecord.period.id, blockId: storedBlock.id, storedRecord, renderedRecord, comparison });
    }
  }
  return reviews;
}

const esc = (value: unknown): string => String(value ?? "")
  .replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
  .replace(/"/g, "&quot;").replace(/'/g, "&#39;");

export function renderReportReview(reviews: ReportBlockReview[], recordCount: number): string {
  const blocks = reviews.map(({ periodId, blockId, storedRecord, renderedRecord, comparison }, index) => {
    const hunks = comparison.hunks.length
      ? `<details class="diff"><summary>Show indexed change${comparison.hunks.length === 1 ? "" : "s"} (${comparison.hunks.length})</summary>${comparison.hunks.map((hunk, i) => `<div class="hunk"><h4>Change ${i + 1}</h4><div class="columns"><pre><strong>Stored</strong>\n${esc(hunk.before || "(empty)")}</pre><pre><strong>Rendered</strong>\n${esc(hunk.after || "(empty)")}</pre></div></div>`).join("")}</details>`
      : "";
    return `<article class="review" data-review="${index}"><p class="meta">${esc(periodId)} · ${esc(blockId)}</p>
<div class="columns"><section><h3>Stored wording</h3><pre>${esc(comparison.storedText)}</pre></section><section><h3>Rendered wording</h3><pre>${esc(comparison.renderedText)}</pre></section></div>${hunks}
<form data-report-resolution data-stored="${esc(storedRecord.id)}" data-rendered="${esc(renderedRecord.id)}" data-block="${esc(blockId)}">
<fieldset><legend>Choose what to keep</legend>
<label><input type="radio" name="strategy-${index}" value="stored" required> Keep stored wording</label>
<label><input type="radio" name="strategy-${index}" value="rendered"> Use rendered wording</label>
<label><input type="radio" name="strategy-${index}" value="edited"> Use my edited wording</label>
<label><input type="radio" name="strategy-${index}" value="combined"> Combine the two (write the result below)</label></fieldset>
<label>Edited or combined wording<textarea name="resolvedText" rows="5" placeholder="Required for edited or combined choices"></textarea></label>
<button type="submit">Record resolution</button><span class="status" role="status"></span></form></article>`;
  }).join("");
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Report review</title><style>
body{font:16px system-ui,sans-serif;max-width:70rem;margin:2rem auto;padding:0 1rem;background:#0e1117;color:#e6edf3}a{color:#58a6ff}.muted,.meta{color:#9da7b1}.review{border:1px solid #30363d;border-radius:.6rem;padding:1rem;margin:1rem 0;background:#11161d}.columns{display:grid;grid-template-columns:repeat(auto-fit,minmax(16rem,1fr));gap:.8rem}pre,textarea{white-space:pre-wrap;overflow-wrap:anywhere;box-sizing:border-box;width:100%;background:#0d1117;color:#e6edf3;border:1px solid #30363d;border-radius:.4rem;padding:.7rem;min-height:5rem}fieldset{display:flex;gap:.7rem;flex-wrap:wrap;border:1px solid #30363d;border-radius:.4rem;margin:1rem 0;padding:.8rem}fieldset label{display:block}textarea{min-height:7rem}button{padding:.65rem 1rem;background:#1f6feb;color:white;border:0;border-radius:.4rem;cursor:pointer}.status{margin-left:.7rem;color:#3fb950}.hunk{border-top:1px solid #30363d}
</style></head><body><h1>Report conflict review</h1><p class="muted">${recordCount} report versions available. Existing wording is never replaced automatically: compare both versions, choose a resolution, then submit it as a new append-only report version.</p>
${blocks || `<p>No comparable report-block conflicts found. Conflicts are shown when two retained versions share a period, template, and block ID but contain different prose.</p>`}
<script>document.querySelectorAll('[data-report-resolution]').forEach(function(form){form.addEventListener('submit',async function(event){event.preventDefault();var status=form.querySelector('.status');var strategy=form.querySelector('input[type=radio]:checked');var text=form.elements.resolvedText.value;if(!strategy){status.textContent='Choose a resolution first.';return;}if((strategy.value==='edited'||strategy.value==='combined')&&!text.trim()){status.textContent='Write the edited or combined wording first.';return;}var button=form.querySelector('button');button.disabled=true;status.textContent='Saving…';try{var response=await fetch('/api/reports/resolve',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({storedRecordId:form.dataset.stored,renderedRecordId:form.dataset.rendered,blockId:form.dataset.block,strategy:strategy.value,...((strategy.value==='edited'||strategy.value==='combined')?{resolvedText:text}:{})})});var body=await response.json().catch(function(){return {};});if(!response.ok)throw new Error(body.error||('HTTP '+response.status));status.textContent='Resolution appended. Reload to review the updated report.';}catch(error){status.textContent='Not saved: '+error.message;button.disabled=false;}});});</script></body></html>`;
}
