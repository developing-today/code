package dashboard

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestDashboardHTMLElicitationRefreshPreservesInput guards issue #3223
// behaviorally: it runs the dashboard's real <script> under node with a
// minimal DOM stub and drives loadElicitations through refresh
// transitions, asserting that (a) a refresh whose pending set is
// unchanged -- including one where only volatile per-listing metadata
// (ttlRemainingMs, lastTouched) moved -- does NOT re-render, so typed
// answers and focus survive untouched; and (b) a refresh whose set did
// change re-renders and restores in-progress values and focus keyed by
// (request id, field name).
//
// This replaces a substring-pinning test that could not fail when the
// preservation mechanism broke (Codex review on PR #3237): removing the
// value capture or breaking the signature left every assertion green.
// The signature checks here fail against a whole-response
// JSON.stringify signature, because the real /v1/elicit payload's
// ttlRemainingMs changes on every list. Skips when node is unavailable,
// following the python3 precedent in internal/codegen.
func TestDashboardHTMLElicitationRefreshPreservesInput(t *testing.T) {
	node, err := exec.LookPath("node")
	if err != nil {
		t.Skip("node not available")
	}
	html := HTML("test")
	start := strings.Index(html, "<script>")
	end := strings.Index(html, "</script>")
	if start < 0 || end <= start {
		t.Fatalf("dashboard HTML has no script block")
	}
	script := html[start+len("<script>") : end]

	dir := t.TempDir()
	main := filepath.Join(dir, "main.js")
	program := elicitHarnessPrelude + script + elicitHarnessScenario
	if err := os.WriteFile(main, []byte(program), 0o644); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	out, err := exec.CommandContext(ctx, node, main).CombinedOutput()
	if err != nil {
		t.Fatalf("elicitation behavioral test failed: %v\n%s", err, out)
	}
	if !strings.Contains(string(out), "BEHAVIORAL-OK") {
		t.Fatalf("elicitation behavioral test did not report success:\n%s", out)
	}
}

// elicitHarnessPrelude installs the minimal DOM the dashboard script
// touches (getElementById, activeElement, querySelectorAll) plus
// offline fetch and no-op interval timers, before the page script runs.
const elicitHarnessPrelude = `
var __failures = [];
function __check(name, cond) {
  if (!cond) { __failures.push(name); console.error('FAIL: ' + name); }
  else { console.log('ok: ' + name); }
}
process.on('unhandledRejection', function () {});
function __El(tag) {
  this.tagName = tag;
  this.id = '';
  this.value = '';
  this.innerText = '';
  this.textContent = '';
  this.style = {};
  this.children = [];
  this.renderCount = 0;
  this._html = '';
  this._fields = null;
  this.classList = { add: function () {}, remove: function () {}, toggle: function () {}, contains: function () { return false; } };
  var self = this;
  Object.defineProperty(this, 'innerHTML', {
    get: function () { return self._html; },
    set: function (v) { self._html = String(v); self._fields = null; self.renderCount++; }
  });
}
__El.prototype.getAttribute = function () { return null; };
__El.prototype.setAttribute = function () {};
__El.prototype.addEventListener = function () {};
__El.prototype.removeEventListener = function () {};
__El.prototype.appendChild = function (c) { return c; };
__El.prototype.focus = function () {};
__El.prototype.remove = function () {};
__El.prototype.contains = function (el) { return el === this || this._fieldEls().indexOf(el) >= 0; };
__El.prototype.querySelectorAll = function (sel) { return sel === '[data-field]' ? this._fieldEls() : []; };
__El.prototype.querySelector = function () { return null; };
__El.prototype.closest = function () { return null; };
__El.prototype._fieldEls = function () {
  if (this._fields) return this._fields;
  var els = [];
  var re = /<div class="elicit-card" id="elicit-card-([^"]*)">|<(?:input|select)\b[^>]*>/g;
  var m; var card = '';
  while ((m = re.exec(this._html)) !== null) {
    if (m[1] !== undefined) { card = m[1]; continue; }
    var tag = m[0];
    var fm = /data-field="([^"]*)"/.exec(tag);
    if (!fm) continue;
    var idm = / id="([^"]*)"/.exec(tag);
    var vm = / value="([^"]*)"/.exec(tag);
    var el = new __El('field');
    el.id = idm ? idm[1] : '';
    el.__field = fm[1];
    el.__card = card;
    el.value = vm ? vm[1] : '';
    el.selectionStart = 0; el.selectionEnd = 0;
    el.getAttribute = function (n) { return n === 'data-field' ? this.__field : (n === 'id' ? this.id : null); };
    el.closest = function () { var c = new __El('div'); c.id = 'elicit-card-' + this.__card; return c; };
    el.focus = function () { document.activeElement = this; };
    el.setSelectionRange = function (a, b) { this.selectionStart = a; this.selectionEnd = b; };
    els.push(el);
  }
  this._fields = els;
  return els;
};
var __byId = {};
var document = {
  activeElement: null,
  getElementById: function (id) {
    if (!__byId[id]) { var el = new __El('div'); el.id = id; __byId[id] = el; }
    return __byId[id];
  },
  querySelectorAll: function () { return []; },
  querySelector: function () { return null; },
  createElement: function (tag) { return new __El(tag); },
  addEventListener: function () {},
  body: null
};
document.body = new __El('body');
var fetch = function () { return Promise.reject(new Error('offline in test')); };
var setInterval = function () { return 0; };
`

// elicitHarnessScenario drives the page's own loadElicitations and
// elicitSignature through the refresh transitions issue #3223 is about.
const elicitHarnessScenario = `
;(async function () {
  function wait(ms) { return new Promise(function (r) { setTimeout(r, ms); }); }
  await wait(30);
  var schemaA = { type: 'object', properties: { name: { type: 'string', title: 'Name' }, count: { type: 'number' } }, required: ['name'] };
  var schemaB = { type: 'object', properties: { ok: { type: 'boolean', title: 'OK?' } } };
  function req(id, ttl, schema, touched) {
    return { id: id, mode: 'form', server: 'srv', tool: 'tool', session: 'sess', message: 'answer please',
      requestedSchema: typeof schema === 'string' ? schema : JSON.stringify(schema),
      created: '2026-01-01T00:00:00Z', expiresAt: '2026-01-01T01:00:00Z',
      lastTouched: touched || '2026-01-01T00:00:00Z', ttlRemainingMs: ttl, state: 'pending', audience: 'operator' };
  }
  __check('signature stable across ttlRemainingMs drift',
    elicitSignature([req('r1', 5000, schemaA)]) === elicitSignature([req('r1', 3000, schemaA)]));
  __check('signature stable across lastTouched drift',
    elicitSignature([req('r1', 5000, schemaA, '2026-01-01T00:00:00Z')]) === elicitSignature([req('r1', 5000, schemaA, '2026-01-01T00:04:00Z')]));
  __check('signature equal for string and object schema forms',
    elicitSignature([req('r1', 5000, schemaA)]) === elicitSignature([{ id: 'r1', requestedSchema: schemaA }]));
  var schemaA2 = JSON.parse(JSON.stringify(schemaA)); schemaA2.properties.extra = { type: 'string' };
  __check('signature changes when a field is added',
    elicitSignature([req('r1', 5000, schemaA)]) !== elicitSignature([req('r1', 5000, schemaA2)]));
  __check('signature changes when the request id changes',
    elicitSignature([req('r1', 5000, schemaA)]) !== elicitSignature([req('r9', 5000, schemaA)]));
  __check('signature changes when a set member is added',
    elicitSignature([req('r1', 5000, schemaA)]) !== elicitSignature([req('r1', 5000, schemaA), req('r2', 5000, schemaB)]));

  var list = document.getElementById('elicit-list');
  list.innerHTML = '';
  lastElicitSignature = null;
  fetchJSON = function () { return Promise.resolve([req('r1', 5000, schemaA), req('r2', 5000, schemaB)]); };
  await loadElicitations();
  var rendersAfterFirst = list.renderCount;
  __check('first load renders the pending set', rendersAfterFirst > 0 && list.innerHTML.indexOf('elicit-card-r1') >= 0);
  function field(id) {
    var els = list.querySelectorAll('[data-field]');
    for (var i = 0; i < els.length; i++) { if (els[i].id === id) return els[i]; }
    return null;
  }
  var nameEl = field('elicit-r1-name');
  __check('rendered field for r1/name exists', !!nameEl);
  nameEl.value = 'typed-answer';
  nameEl.focus();
  fetchJSON = function () { return Promise.resolve([req('r1', 2500, schemaA, '2026-01-01T00:03:00Z'), req('r2', 2500, schemaB)]); };
  await loadElicitations();
  __check('volatile-only refresh skips the re-render', list.renderCount === rendersAfterFirst);
  __check('typed answer survives the volatile-only refresh', field('elicit-r1-name') === nameEl && nameEl.value === 'typed-answer');
  fetchJSON = function () { return Promise.resolve([req('r1', 2000, schemaA), req('r2', 2000, schemaB), req('r3', 2000, schemaB)]); };
  await loadElicitations();
  __check('changed set re-renders', list.renderCount === rendersAfterFirst + 1 && list.innerHTML.indexOf('elicit-card-r3') >= 0);
  var restored = field('elicit-r1-name');
  __check('typed answer is restored across a forced re-render', !!restored && restored !== nameEl && restored.value === 'typed-answer');
  __check('focus is restored across a forced re-render', document.activeElement === restored);
  if (__failures.length) { console.error('BEHAVIORAL-FAILED: ' + __failures.join('; ')); process.exit(1); }
  console.log('BEHAVIORAL-OK');
  process.exit(0);
})().catch(function (e) { console.error('BEHAVIORAL-ERROR: ' + (e && e.stack || e)); process.exit(1); });
`
