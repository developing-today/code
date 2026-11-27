// render.ts split (#13051): DNS register view.
// render.ts re-exports every public name from here; callers keep importing "./render.ts".

import { summarizeDns } from "./dns.ts";
import type { DnsEntry, DnsSection, Provenance, ProvenanceDetail } from "./model.ts";
import { dnsAnchor } from "./search-view.ts";

import { esc, fmtDate, freshnessChips, layout } from "./render-core.ts";

// --- DNS register page (#906 §1, data from #902) -----------------------------

function dnsTable(entries: DnsEntry[], renewingSoon: Set<DnsEntry>, nextRenewal?: DnsEntry | null): string {
  const rows = entries
    .map(
      (e) =>
        `<tr${e === nextRenewal ? ' id="dns-next-renewal"' : ""} data-dns-row data-dns-status="${esc(e.status)}" data-dns-renewing-soon="${renewingSoon.has(e)}"><td><strong id="${esc(dnsAnchor(e.domain))}">${esc(e.domain)}</strong></td><td>${esc(e.renewal)}</td><td>${e.priceUsd != null ? `$${e.priceUsd.toFixed(2)}` : "—"}</td><td>${esc(e.autoRenew)}</td><td>${esc(e.plan)}</td><td>${esc(e.review)}</td></tr>`,
    )
    .join("");
  return `<div class="tblwrap"><table id="dns-register-table"><caption>Domain register</caption><thead><tr><th scope="col">Domain</th><th scope="col">Next renewal</th><th scope="col">Price (list)</th><th scope="col">Auto-renew</th><th scope="col">Plan / status</th><th scope="col">Review?</th></tr></thead><tbody>${rows || '<tr><td colspan="6" class="muted">none</td></tr>'}</tbody></table></div>`;
}

const DNS_FILTER_SCRIPT = `<script>
(function () {
  var buttons = document.querySelectorAll("[data-dns-filter]");
  var rows = document.querySelectorAll("[data-dns-row]");
  buttons.forEach(function (button) {
    button.addEventListener("click", function () {
      var filter = button.getAttribute("data-dns-filter");
      buttons.forEach(function (candidate) {
        var selected = candidate === button;
        candidate.classList.toggle("primary", selected);
        candidate.setAttribute("aria-pressed", String(selected));
      });
      rows.forEach(function (row) {
        var matches = filter === "all" || (filter === "renewing-soon"
          ? row.getAttribute("data-dns-renewing-soon") === "true"
          : row.getAttribute("data-dns-status") === filter);
        row.hidden = !matches;
      });
    });
  });
  var nextLink = document.querySelector("[data-dns-next-link]");
  if (nextLink) {
    nextLink.addEventListener("click", function () {
      buttons.forEach(function (candidate) {
        var selected = candidate.getAttribute("data-dns-filter") === "all";
        candidate.classList.toggle("primary", selected);
        candidate.setAttribute("aria-pressed", String(selected));
      });
      rows.forEach(function (row) { row.hidden = false; });
    });
  }
})();
</script>`;

export function renderDns(section: DnsSection | null, provenance: Provenance, detail?: ProvenanceDetail): string {
  const chips = detail ? freshnessChips(detail, ["dns"]) : "";
  if (!section) {
    return layout(
      "DNS register",
      `<h1>DNS / domain register</h1><div class="banner">The register has not reached the dashboard yet — the fleet-feed collector embeds it from #902's register file, and no section is in the current feed. The register of record is the file the #902 pipeline maintains; this page only renders it.</div>`,
      provenance,
      "/dns",
      "",
      chips,
    );
  }
  const s = summarizeDns(section);
  const next = s.nextRenewal;
  const soonTotal = Math.round(s.renewingSoon.reduce((sum, e) => sum + (e.priceUsd ?? 0), 0) * 100) / 100;
  const renewingSoon = new Set(s.renewingSoon);
  const filters = [
    ["all", "All", section.entries.length],
    ["active", "Active", s.active.length],
    ["earmarked", "Earmarked", s.earmarked.length],
    ["unused", "Unused", s.unused.length],
    ["renewing-soon", "Renewing soon", s.renewingSoon.length],
  ] as const;
  const filterChips = filters
    .map(
      ([filter, label, count], index) =>
        `<button type="button" class="qchip dns-filter${index === 0 ? " primary" : ""}" data-dns-filter="${filter}" aria-pressed="${index === 0}">${label} (${count})</button>`,
    )
    .join("");
  const body = `<h1>DNS / domain register</h1>
<p class="muted">${section.entries.length} domains · $${section.totalAnnualListUsd.toFixed(2)}/yr at list renewal prices · register parsed ${fmtDate(section.generatedAt)} (${esc(section.source)}). The register is maintained by the #902 pipeline — this page renders it, never edits it. Machine-first: <a href="/api/dns.json">/api/dns.json</a>.</p>
<div class="board">
<div class="card"><strong>Next renewal</strong><br>${
    next
      ? `${esc(next.renewal)}${next.priceUsd != null ? ` · $${next.priceUsd.toFixed(2)}` : ""}<br><span class="muted">See <a href="#dns-next-renewal" data-dns-next-link>its row in the renewal-sorted register</a>.</span>`
      : "—"
  }</div>
<div class="card"><strong>Renewing in 45 days</strong><br>${s.renewingSoon.length} domain${s.renewingSoon.length === 1 ? "" : "s"} · $${soonTotal.toFixed(2)} at list price</div>
<div class="card"><strong>Auto-renew</strong><br>${s.autoRenew.on} ON · ${s.autoRenew.off} OFF · ${s.autoRenew.unknown} unknown</div>
<div class="card"><strong>Portfolio shape</strong><br>${s.active.length} in use · ${s.earmarked.length} unused but earmarked · ${s.unused.length} completely unused</div>
</div>
<h2>Domain register (${section.entries.length})</h2>
<div class="chips" role="group" aria-label="Filter domains by status">${filterChips}</div>
${dnsTable(section.entries, renewingSoon, next)}`;
  return layout("DNS register", body, provenance, "/dns", DNS_FILTER_SCRIPT, chips);
}
