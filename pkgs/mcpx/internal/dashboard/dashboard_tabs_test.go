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

// TestDashboardHTMLTabsAriaAndKeyboard guards the Operator Views tablist
// on issue #6214 (ARIA tabs pattern, completed in PR #6693) behaviorally:
// it runs the dashboard's real <script> under node with a tab-aware DOM
// stub and drives the tablist's own keydown handler, asserting that
// Arrow/Home/End keys move focus and selection together, that
// aria-selected and the roving tabindex stay in sync with the selected
// tab, and that the shared panel's aria-labelledby names the active tab.
//
// Oracle: the WAI-ARIA tabs pattern — exactly one tab is selected and in
// the sequential focus order (tabindex "0"); all others have
// aria-selected "false" and tabindex "-1". The static markup checks pin
// the initial state the script starts from.
//
// This exists because the elicitation harness's querySelector stub
// returns null, so it can never attach or exercise this handler
// (Codex P1 on PR #6693). Against the pre-#6214 page the scenario fails:
// no keydown handler is registered and no aria state is ever written.
// Skips when node is unavailable, like the elicitation test.
func TestDashboardHTMLTabsAriaAndKeyboard(t *testing.T) {
	node, err := exec.LookPath("node")
	if err != nil {
		t.Skip("node not available")
	}
	html := HTML("test")

	// Initial markup state (the script's starting point).
	for _, want := range []string{
		`id="tab-browse"`,
		`aria-selected="true"`,
		`tabindex="0"`,
		`id="view-body" role="tabpanel" aria-labelledby="tab-browse"`,
	} {
		if !strings.Contains(html, want) {
			t.Fatalf("dashboard HTML missing initial tabs state %q", want)
		}
	}
	if got := strings.Count(html, `role="tab"`); got != 6 {
		t.Fatalf("dashboard HTML has %d role=tab tabs, want 6", got)
	}
	if got := strings.Count(html, `tabindex="-1"`); got != 5 {
		t.Fatalf("dashboard HTML has %d initially unselected tabindex=-1 tabs, want 5", got)
	}

	start := strings.Index(html, "<script>")
	end := strings.Index(html, "</script>")
	if start < 0 || end <= start {
		t.Fatalf("dashboard HTML has no script block")
	}
	script := html[start+len("<script>") : end]

	dir := t.TempDir()
	main := filepath.Join(dir, "main.js")
	program := tabsHarnessPrelude + script + tabsHarnessScenario
	if err := os.WriteFile(main, []byte(program), 0o644); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	out, err := exec.CommandContext(ctx, node, main).CombinedOutput()
	if err != nil {
		t.Fatalf("tabs behavioral test failed: %v\n%s", err, out)
	}
	if !strings.Contains(string(out), "BEHAVIORAL-OK") {
		t.Fatalf("tabs behavioral test did not report success:\n%s", out)
	}
}

// tabsHarnessPrelude installs a DOM stub rich enough for the page
// script's top level plus a faithful tablist: six tab elements whose
// classList/attributes/focus the scenario can observe, and a .view-tabs
// container that records the keydown handler the page registers.
const tabsHarnessPrelude = `
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
  this.innerHTML = '';
  this.style = {};
  this.children = [];
  this.options = [];
  this.attrs = {};
  var self = this;
  this.classList = {
    add: function () {}, remove: function () {},
    toggle: function (cls, on) { if (cls === 'active') self._active = !!on; },
    contains: function () { return false; }
  };
}
__El.prototype.getAttribute = function (n) { return n in this.attrs ? this.attrs[n] : null; };
__El.prototype.setAttribute = function (n, v) { this.attrs[n] = String(v); };
__El.prototype.addEventListener = function () {};
__El.prototype.removeEventListener = function () {};
__El.prototype.appendChild = function (c) { return c; };
__El.prototype.insertAdjacentHTML = function () {};
__El.prototype.focus = function () { document.activeElement = this; };
__El.prototype.remove = function () {};
__El.prototype.querySelectorAll = function () { return []; };
__El.prototype.querySelector = function () { return null; };
__El.prototype.closest = function () { return null; };
function __Tab(view) {
  __El.call(this, 'button');
  this.dataset = { view: view };
}
__Tab.prototype = Object.create(__El.prototype);
__Tab.prototype.constructor = __Tab;
var __tabViews = ['browse', 'log', 'stats', 'servers', 'sessions', 'storage'];
var __tabs = __tabViews.map(function (v) {
  var t = new __Tab(v);
  t.setAttribute('aria-selected', v === 'browse' ? 'true' : 'false');
  t.setAttribute('tabindex', v === 'browse' ? '0' : '-1');
  if (v === 'browse') t._active = true;
  return t;
});
var __keydownHandler = null;
var __tabsEl = new __El('div');
__tabsEl.addEventListener = function (type, fn) { if (type === 'keydown') __keydownHandler = fn; };
var __byId = {};
var document = {
  activeElement: null,
  getElementById: function (id) {
    if (!__byId[id]) { var el = new __El('div'); el.id = id; __byId[id] = el; }
    return __byId[id];
  },
  querySelectorAll: function (sel) { return sel === '.view-tab' ? __tabs : []; },
  querySelector: function (sel) { return sel === '.view-tabs' ? __tabsEl : null; },
  createElement: function (tag) { return new __El(tag); },
  addEventListener: function () {},
  body: null
};
document.body = new __El('body');
var fetch = function () { return Promise.reject(new Error('offline in test')); };
var setInterval = function () { return 0; };
`

// tabsHarnessScenario drives the captured keydown handler exactly as a
// keyboard user would: focus a tab, send keys, observe focus, selection
// attributes, roving tabindex, panel labelling, and currentView.
const tabsHarnessScenario = `
;(async function () {
  function wait(ms) { return new Promise(function (r) { setTimeout(r, ms); }); }
  await wait(30);
  function key(k) {
    var prevented = false;
    __keydownHandler({ key: k, preventDefault: function () { prevented = true; } });
    return prevented;
  }
  function selectedIdx() {
    for (var i = 0; i < __tabs.length; i++) {
      if (__tabs[i].getAttribute('aria-selected') === 'true') return i;
    }
    return -1;
  }
  function checkSync(name, idx) {
    __check(name + ': selection index', selectedIdx() === idx);
    __check(name + ': currentView', currentView === __tabViews[idx]);
    __check(name + ': focus on selected tab', document.activeElement === __tabs[idx]);
    __check(name + ': panel labelled by active tab',
      document.getElementById('view-body').getAttribute('aria-labelledby') === 'tab-' + __tabViews[idx]);
    var ok = true;
    for (var i = 0; i < __tabs.length; i++) {
      if (__tabs[i].getAttribute('tabindex') !== (i === idx ? '0' : '-1')) ok = false;
      if (__tabs[i].getAttribute('aria-selected') !== (i === idx ? 'true' : 'false')) ok = false;
    }
    __check(name + ': roving tabindex + aria-selected sync', ok);
  }
  __check('page registered a keydown handler on the tablist', typeof __keydownHandler === 'function');
  __tabs[0].focus();
  __check('ArrowRight is handled', key('ArrowRight') === true);
  checkSync('after ArrowRight', 1);
  key('ArrowRight');
  checkSync('after second ArrowRight', 2);
  key('ArrowLeft');
  checkSync('after ArrowLeft', 1);
  key('End');
  checkSync('after End', 5);
  key('ArrowRight');
  checkSync('after ArrowRight wraps past End', 0);
  key('ArrowLeft');
  checkSync('after ArrowLeft wraps before Home', 5);
  key('Home');
  checkSync('after Home', 0);
  __check('unrelated key is not handled', key('a') === false);
  checkSync('after unrelated key', 0);
  if (__failures.length) { console.error('BEHAVIORAL-FAILED: ' + __failures.join('; ')); process.exit(1); }
  console.log('BEHAVIORAL-OK');
  process.exit(0);
})().catch(function (e) { console.error('BEHAVIORAL-ERROR: ' + (e && e.stack || e)); process.exit(1); });
`
