package daemon

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
)

// #1933 slice 3: a virtual namespace mounts tools from several
// upstream servers under one namespace. The oracle for the listing
// tests is the seeded catalogues of the two upstreams; the oracle
// for the call test is the upstream that actually received the
// call, which reports its own name, the tool name and the exact
// arguments it was handed.

// echoUpstream is a minimal HTTP MCP server: a fixed tools/list and
// a tools/call that echoes who was called with what.
type echoUpstream struct {
	*httptest.Server
	name string
}

func newEchoUpstream(t *testing.T, name string, tools []mcpclient.Tool) *echoUpstream {
	t.Helper()
	u := &echoUpstream{name: name}
	u.Server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var req struct {
			ID     json.RawMessage `json:"id"`
			Method string          `json:"method"`
			Params struct {
				Name      string          `json:"name"`
				Arguments json.RawMessage `json:"arguments"`
			} `json:"params"`
		}
		_ = json.NewDecoder(r.Body).Decode(&req)
		reply := func(result any) {
			w.Header().Set("Content-Type", "application/json")
			_ = json.NewEncoder(w).Encode(map[string]any{"jsonrpc": "2.0", "id": req.ID, "result": result})
		}
		switch req.Method {
		case "server/discover":
			w.Header().Set("Content-Type", "application/json")
			_ = json.NewEncoder(w).Encode(map[string]any{"jsonrpc": "2.0", "id": req.ID,
				"error": map[string]any{"code": -32601, "message": "method not found"}})
		case "initialize":
			reply(map[string]any{
				"protocolVersion": "2025-03-26",
				"capabilities":    map[string]any{"tools": map[string]any{}},
				"serverInfo":      map[string]any{"name": name, "version": "dev"},
			})
		case "notifications/initialized":
			w.WriteHeader(http.StatusAccepted)
		case "tools/list":
			reply(map[string]any{"tools": tools})
		case "tools/call":
			text := fmt.Sprintf(`{"server":%q,"tool":%q,"arguments":%s}`, name, req.Params.Name, req.Params.Arguments)
			reply(map[string]any{"content": []map[string]any{{"type": "text", "text": text}}})
		default:
			reply(map[string]any{})
		}
	}))
	t.Cleanup(u.Close)
	return u
}

func echoTool() mcpclient.Tool {
	return mcpclient.Tool{Name: "echo", Description: "echoes",
		InputSchema: json.RawMessage(`{"type":"object","properties":{"fixed":{"type":"string"},"q":{"type":"string"}}}`)}
}

func searchTool() mcpclient.Tool {
	return mcpclient.Tool{Name: "search", Description: "searches",
		InputSchema: json.RawMessage(`{"type":"object","properties":{"q":{"type":"string"}}}`)}
}

// virtualRegistry builds two upstreams that both expose an "echo"
// and a "search" tool, plus a virtual namespace mounting all four
// under distinct view names -- the collision case: the same tool
// name on two servers, disambiguated by the view names the author
// chose, which is the deterministic rule slice 3 documents.
func virtualRegistry(t *testing.T) (*Registry, *echoUpstream, *echoUpstream) {
	t.Helper()
	tools := []mcpclient.Tool{echoTool(), searchTool()}
	alpha := newEchoUpstream(t, "alpha", tools)
	beta := newEchoUpstream(t, "beta", tools)
	dir := t.TempDir()
	cfg := &config.Config{MCPServers: map[string]*config.Server{
		"alpha": {Name: "alpha", URL: alpha.URL},
		"beta":  {Name: "beta", URL: beta.URL},
		"combo": {Name: "combo", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
			"alphaEcho":   {From: "alpha.echo", Pin: map[string]any{"fixed": "A"}, Hide: []string{"fixed"}},
			"betaEcho":    {From: "beta.echo"},
			"alphaSearch": {From: "alpha.search"},
			"betaSearch":  {From: "beta.search"},
			"echoAgain":   {From: "alpha.echo"},
		}}},
	}}
	r, err := NewRegistry(cfg, Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	return r, alpha, beta
}

func TestVirtualNamespaceListsMountedTools(t *testing.T) {
	r, _, _ := virtualRegistry(t)
	view, ok := r.View("combo")
	if !ok || !view.Virtual() {
		t.Fatal("combo must be a virtual view")
	}
	now := time.Now()
	for _, srv := range []string{"alpha", "beta"} {
		r.pools[srv].SetSchemas([]mcpclient.Tool{echoTool(), searchTool()}, nil, nil, "",
			now, now.Add(10*time.Minute), false)
	}
	var names []string
	var echoSchema string
	for _, ti := range r.Tools([]string{"combo"}) {
		if ti.Server != "combo" {
			continue
		}
		names = append(names, ti.Tool)
		if ti.Tool == "alphaEcho" {
			echoSchema = string(ti.InputSchema)
		}
	}
	want := []string{"alphaEcho", "alphaSearch", "betaEcho", "betaSearch", "echoAgain"}
	if strings.Join(names, ",") != strings.Join(want, ",") {
		t.Fatalf("combo must list exactly its mounts, sorted: got %v want %v", names, want)
	}
	if strings.Contains(echoSchema, "fixed") {
		t.Errorf("the mount's pinned property must be hidden from its schema: %s", echoSchema)
	}
	if !strings.Contains(echoSchema, `"q"`) {
		t.Errorf("unpinned properties stay visible: %s", echoSchema)
	}
	// The namespace row counts the mounted tools, not an own catalogue.
	for _, ns := range r.Namespaces(config.Profile{}) {
		if ns.Server == "combo" && ns.Tools != len(want) {
			t.Errorf("combo namespace must report %d tools, got %d", len(want), ns.Tools)
		}
	}
}

func TestVirtualCallRoutesToOwningServerWithPins(t *testing.T) {
	r, _, _ := virtualRegistry(t)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	cc := config.CallContext{}

	call := func(tool string, args map[string]any) map[string]any {
		t.Helper()
		res, err := r.Call(ctx, "combo", tool, cc, args)
		if err != nil {
			t.Fatalf("call combo.%s: %v", tool, err)
		}
		var out struct {
			Content []struct {
				Text string `json:"text"`
			} `json:"content"`
		}
		if err := json.Unmarshal(res, &out); err != nil || len(out.Content) != 1 {
			t.Fatalf("call result shape: %v %s", err, res)
		}
		var echo map[string]any
		if err := json.Unmarshal([]byte(out.Content[0].Text), &echo); err != nil {
			t.Fatalf("echo payload: %v", err)
		}
		return echo
	}

	got := call("alphaEcho", map[string]any{"q": "x", "fixed": "caller-value"})
	if got["server"] != "alpha" || got["tool"] != "echo" {
		t.Errorf("alphaEcho must route to alpha.echo: %v", got)
	}
	callArgs, _ := got["arguments"].(map[string]any)
	if callArgs["fixed"] != "A" {
		t.Errorf("the mount's pin must win over the caller's value: %v", callArgs)
	}
	if callArgs["q"] != "x" {
		t.Errorf("unpinned arguments pass through: %v", callArgs)
	}

	got = call("betaEcho", map[string]any{"q": "y"})
	if got["server"] != "beta" || got["tool"] != "echo" {
		t.Errorf("betaEcho must route to beta.echo: %v", got)
	}

	// A tool the namespace does not mount is not callable through it.
	if _, err := r.Call(ctx, "combo", "echo", cc, map[string]any{}); err == nil {
		t.Error("the bare upstream name must not be callable through the virtual namespace")
	}
}

func TestVirtualValidationNamesUnknownMountTarget(t *testing.T) {
	r, _, _ := virtualRegistry(t)
	now := time.Now()
	// alpha's catalogue lacks the mounted "echo"; beta's is complete,
	// so the assembled catalogue is non-empty and validation runs.
	r.pools["alpha"].SetSchemas([]mcpclient.Tool{searchTool()}, nil, nil, "", now, now.Add(10*time.Minute), false)
	r.pools["beta"].SetSchemas([]mcpclient.Tool{echoTool(), searchTool()}, nil, nil, "", now, now.Add(10*time.Minute), false)
	err := r.validateViews()
	if err == nil || !strings.Contains(err.Error(), "alpha.echo") {
		t.Fatalf("validation must name the unknown mount target, got %v", err)
	}
}

// A mount the namespace hides (here via excludeTools) is hidden
// from calls too, on every entry point -- visibility is enforced
// before delegation, not just in listings (Codex P1 on PR #8632).
func TestVirtualHiddenMountIsNotCallable(t *testing.T) {
	tools := []mcpclient.Tool{echoTool(), searchTool()}
	alpha := newEchoUpstream(t, "alpha", tools)
	beta := newEchoUpstream(t, "beta", tools)
	dir := t.TempDir()
	cfg := &config.Config{MCPServers: map[string]*config.Server{
		"alpha": {Name: "alpha", URL: alpha.URL},
		"beta":  {Name: "beta", URL: beta.URL},
		"combo": {Name: "combo", Mcpx: &config.Extras{
			ExcludeTools: []string{"betaEcho"},
			ToolViews: map[string]config.ToolView{
				"alphaEcho": {From: "alpha.echo"},
				"betaEcho":  {From: "beta.echo"},
			}}},
	}}
	r, err := NewRegistry(cfg, Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	cc := config.CallContext{}
	if _, err := r.Call(ctx, "combo", "betaEcho", cc, map[string]any{}); err == nil {
		t.Error("an excluded mount must not be callable through Call")
	}
	if _, err := r.CallAsk(ctx, "ask-hidden", "combo", "betaEcho", cc, map[string]any{}); err == nil {
		t.Error("an excluded mount must not be callable through CallAsk")
	}
	if _, err := r.CallReporting(ctx, "rep-hidden", "combo", "betaEcho", cc, map[string]any{}); err == nil {
		t.Error("an excluded mount must not be callable through CallReporting")
	}
	// The visible mount still routes on the ask path.
	res, err := r.CallAsk(ctx, "ask-visible", "combo", "alphaEcho", cc, map[string]any{"q": "x"})
	if err != nil {
		t.Fatalf("CallAsk on a visible mount: %v", err)
	}
	if !strings.Contains(string(res), `\"server\":\"alpha\"`) {
		t.Errorf("CallAsk must route the mount to its home server: %s", res)
	}
}

// Mount validation defers per source: with only beta's catalogue
// cached, alpha's mounts are unchecked rather than unknown
// (Codex P2 on PR #8632).
func TestVirtualValidationDefersUncachedSource(t *testing.T) {
	r, _, _ := virtualRegistry(t)
	now := time.Now()
	r.pools["beta"].SetSchemas([]mcpclient.Tool{echoTool(), searchTool()}, nil, nil, "", now, now.Add(10*time.Minute), false)
	if err := r.validateViews(); err != nil {
		t.Fatalf("an uncached source's mounts must defer, got %v", err)
	}
}

// The schema advertised for a mount carries the home server's
// argument rules, because the delegated call enforces them
// (Codex P2 on PR #8632).
func TestVirtualMountedSchemaCarriesSourceArgumentRules(t *testing.T) {
	r, _, _ := virtualRegistry(t)
	set, err := config.CompileArgumentRules([]config.ArgumentRuleSource{
		{Key: "echo", Origin: "test config", Schema: json.RawMessage(`{"properties":{"q":{"maxLength":3}}}`)},
	})
	if err != nil {
		t.Fatal(err)
	}
	r.views["alpha"].ArgumentRules = set
	now := time.Now()
	for _, srv := range []string{"alpha", "beta"} {
		r.pools[srv].SetSchemas([]mcpclient.Tool{echoTool(), searchTool()}, nil, nil, "",
			now, now.Add(10*time.Minute), false)
	}
	for _, ti := range r.Tools([]string{"combo"}) {
		if ti.Tool == "alphaEcho" && !strings.Contains(string(ti.InputSchema), "maxLength") {
			t.Errorf("alpha's rule on echo must be advertised on the mount: %s", ti.InputSchema)
		}
		if ti.Tool == "betaEcho" && strings.Contains(string(ti.InputSchema), "maxLength") {
			t.Errorf("alpha's rule must not leak onto beta's mount: %s", ti.InputSchema)
		}
	}
}

// A virtual namespace row reports cached exactly when its mount
// sources are cached, so a cold source never lets a partial
// assembly present as authoritative (Codex P2 on PR #8632).
func TestVirtualNamespaceCachedFollowsSources(t *testing.T) {
	r, _, _ := virtualRegistry(t)
	cachedOf := func() bool {
		t.Helper()
		for _, ns := range r.Namespaces(config.Profile{}) {
			if ns.Server == "combo" {
				return ns.Cached
			}
		}
		t.Fatal("combo namespace row missing")
		return false
	}
	if cachedOf() {
		t.Fatal("with no source cached, the virtual namespace must not report cached")
	}
	now := time.Now()
	r.pools["alpha"].SetSchemas([]mcpclient.Tool{echoTool(), searchTool()}, nil, nil, "", now, now.Add(10*time.Minute), false)
	if cachedOf() {
		t.Fatal("with one source still cold, the virtual namespace must not report cached")
	}
	r.pools["beta"].SetSchemas([]mcpclient.Tool{echoTool(), searchTool()}, nil, nil, "", now, now.Add(10*time.Minute), false)
	if !cachedOf() {
		t.Fatal("with every source cached, the virtual namespace must report cached")
	}
}

// A mount whose source presently gates the tool out of its own
// visible catalogue (here a per-tool precondition on a secret
// that has not been supplied) is blocked, not
// unknown: validation judges existence against the source's
// declared presentation, so registry creation does not fail for
// a transient gate (Codex P2 on PR #8632).
func TestVirtualValidationToleratesGatedSourceTool(t *testing.T) {
	tools := []mcpclient.Tool{echoTool(), searchTool()}
	alpha := newEchoUpstream(t, "alpha", tools)
	beta := newEchoUpstream(t, "beta", tools)
	dir := t.TempDir()
	cfg := &config.Config{MCPServers: map[string]*config.Server{
		"alpha": {Name: "alpha", URL: alpha.URL, Mcpx: &config.Extras{
			// The exclude entry matters to the mechanism, not
			// the assertion: visibleTools applies a view's
			// VisibleTool filtering (through which the
			// precondition gate acts) only when the view
			// carries an include/exclude list or tool views.
			ExcludeTools: []string{"search"},
			Preconditions: map[string]config.Precondition{
				"echo": {RequiresSecret: "ECHO_TOKEN"},
			}}},
		"beta": {Name: "beta", URL: beta.URL},
		"combo": {Name: "combo", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
			"alphaEcho": {From: "alpha.echo"},
			"betaEcho":  {From: "beta.echo"},
		}}},
	}}
	r, err := NewRegistry(cfg, Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	now := time.Now()
	for _, srv := range []string{"alpha", "beta"} {
		r.pools[srv].SetSchemas(tools, nil, nil, "", now, now.Add(10*time.Minute), false)
	}
	if err := r.validateViews(); err != nil {
		t.Fatalf("a gated mount target must not fail validation: %v", err)
	}
	// The gate is real, not decorative: the assembled listing
	// omits the gated mount (and keeps the ungated one) while
	// validation above accepts it. (A RequiresServer precondition
	// would not discriminate here -- the presentation path passes
	// no activity callback, so only a secret gate hides the tool;
	// and without the exclude entry above, visibleTools would
	// return the source catalogue unfiltered and the gate would
	// never reach the assembly at all.)
	var listed []string
	for _, ti := range r.Tools([]string{"combo"}) {
		listed = append(listed, ti.Tool)
	}
	if strings.Join(listed, ",") != "betaEcho" {
		t.Fatalf("the gated mount must be absent from the listing, the ungated present: %v", listed)
	}
}

// DiagnoseTools derives a virtual namespace's readiness from its
// mount sources: unready sources omit the mounts, ready sources
// report them (Codex P2 on PR #8632).
func TestVirtualDiagnoseToolsFollowsSources(t *testing.T) {
	r, _, _ := virtualRegistry(t)
	comboTools := func() int {
		n := 0
		for _, dt := range r.DiagnoseTools() {
			if dt.Namespace == "combo" {
				n++
			}
		}
		return n
	}
	if got := comboTools(); got != 0 {
		t.Fatalf("with sources uncached, DiagnoseTools must omit the mounts, got %d", got)
	}
	now := time.Now()
	for _, srv := range []string{"alpha", "beta"} {
		r.pools[srv].SetSchemas([]mcpclient.Tool{echoTool(), searchTool()}, nil, nil, "",
			now, now.Add(10*time.Minute), false)
	}
	if got := comboTools(); got != 5 {
		t.Fatalf("with sources cached, DiagnoseTools must report all 5 mounts, got %d", got)
	}
}

// The mounting namespace's own argument rules, keyed by the
// qualified From as the advertised schema carries them, are
// enforced before delegation (Codex P2 on PR #8632).
func TestVirtualMountingViewRulesAreEnforced(t *testing.T) {
	r, _, _ := virtualRegistry(t)
	set, err := config.CompileArgumentRules([]config.ArgumentRuleSource{
		{Key: "alpha.echo", Origin: "test config", Schema: json.RawMessage(`{"properties":{"q":{"maxLength":3}}}`)},
	})
	if err != nil {
		t.Fatal(err)
	}
	r.views["combo"].ArgumentRules = set
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	cc := config.CallContext{}
	if _, err := r.Call(ctx, "combo", "alphaEcho", cc, map[string]any{"q": "toolong"}); err == nil {
		t.Fatal("a call violating the mounting view's rule must be refused before delegation")
	}
	if _, err := r.Call(ctx, "combo", "alphaEcho", cc, map[string]any{"q": "ok"}); err != nil {
		t.Fatalf("a call within the mounting view's rule must pass: %v", err)
	}
	// The rule is keyed to alpha.echo only; beta's mount is free of it.
	if _, err := r.Call(ctx, "combo", "betaEcho", cc, map[string]any{"q": "toolong"}); err != nil {
		t.Fatalf("the alpha.echo rule must not govern beta's mount: %v", err)
	}
}

// A mount may name a tool as an alias of its source exposes it --
// renamed. Existence is judged against the source's declared
// presentation (the renamed name), not the raw upstream name the
// shared pool caches (Codex P2 on PR #8632).
func TestVirtualValidationAcceptsAliasRenamedTarget(t *testing.T) {
	tools := []mcpclient.Tool{echoTool(), searchTool()}
	alpha := newEchoUpstream(t, "alpha", tools)
	beta := newEchoUpstream(t, "beta", tools)
	dir := t.TempDir()
	cfg := &config.Config{MCPServers: map[string]*config.Server{
		"alpha": {Name: "alpha", URL: alpha.URL},
		"beta":  {Name: "beta", URL: beta.URL},
		"alphaView": {Name: "alphaView", AliasOf: "alpha", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
			"renamed": {From: "echo"},
		}}},
		"combo": {Name: "combo", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
			"viaAlias": {From: "alphaView.renamed"},
			"betaEcho": {From: "beta.echo"},
		}}},
	}}
	r, err := NewRegistry(cfg, Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	now := time.Now()
	seeded := map[*pool.Pool]bool{}
	for _, srv := range []string{"alpha", "beta", "alphaView"} {
		if pl := r.pools[srv]; pl != nil && !seeded[pl] {
			seeded[pl] = true
			pl.SetSchemas(tools, nil, nil, "", now, now.Add(10*time.Minute), false)
		}
	}
	if err := r.validateViews(); err != nil {
		t.Fatalf("a mount naming an alias's renamed tool must validate: %v", err)
	}
	// And the mount is callable through the alias's presentation.
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	res, err := r.Call(ctx, "combo", "viaAlias", config.CallContext{}, map[string]any{"q": "x"})
	if err != nil {
		t.Fatalf("call through the alias mount: %v", err)
	}
	if !strings.Contains(string(res), "alpha") {
		t.Errorf("the alias mount must reach alpha: %s", res)
	}
}

// Script preflight sees through a virtual namespace to its mount
// sources: a source waiting on a secret blocks a script that
// references only the namespace, and the source's secret request
// is raised (Codex P2 on PR #8632).
func TestVirtualPreflightSeesMountedSourcesSecrets(t *testing.T) {
	tools := []mcpclient.Tool{echoTool(), searchTool()}
	alpha := newEchoUpstream(t, "alpha", tools)
	beta := newEchoUpstream(t, "beta", tools)
	dir := t.TempDir()
	cfg := &config.Config{MCPServers: map[string]*config.Server{
		"alpha": {Name: "alpha", URL: alpha.URL, Mcpx: &config.Extras{RequiresSecrets: []string{"ALPHA_TOKEN"}}},
		"beta":  {Name: "beta", URL: beta.URL},
		"combo": {Name: "combo", Mcpx: &config.Extras{ToolViews: map[string]config.ToolView{
			"alphaEcho": {From: "alpha.echo"},
			"betaEcho":  {From: "beta.echo"},
		}}},
	}}
	r, err := NewRegistry(cfg, Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	rep := r.Preflight("const x = await tools.combo.alphaEcho({q: 'hi'})")
	if rep.Ready {
		t.Fatal("preflight must report not-ready when a mounted source is missing a secret")
	}
	found := false
	for _, m := range rep.Missing {
		if m.Server == "alpha" && strings.Join(m.Secrets, ",") == "ALPHA_TOKEN" {
			found = true
		}
	}
	if !found {
		t.Fatalf("preflight must name the mounted source and its secret: %+v", rep.Missing)
	}
	if len(rep.Requests) == 0 {
		t.Error("preflight must raise the source's secret request")
	}
}

// The addressed namespace's own blocked state gates a mounted
// call: a virtual namespace that requires a server waiting on a
// secret refuses its mounts even when the mount's home server is
// ready, and raises the dependency's secret request
// (Codex P2 on PR #8632).
func TestVirtualCallRefusedWhenNamespaceDependencyBlocked(t *testing.T) {
	tools := []mcpclient.Tool{echoTool(), searchTool()}
	alpha := newEchoUpstream(t, "alpha", tools)
	beta := newEchoUpstream(t, "beta", tools)
	dir := t.TempDir()
	cfg := &config.Config{MCPServers: map[string]*config.Server{
		"alpha": {Name: "alpha", URL: alpha.URL},
		"beta":  {Name: "beta", URL: beta.URL},
		"auth":  {Name: "auth", URL: beta.URL, Mcpx: &config.Extras{RequiresSecrets: []string{"AUTH_TOKEN"}}},
		"combo": {Name: "combo", Mcpx: &config.Extras{
			RequiresServers: []string{"auth"},
			ToolViews: map[string]config.ToolView{
				"betaEcho": {From: "beta.echo"},
			}}},
	}}
	r, err := NewRegistry(cfg, Paths{State: dir, Cache: dir}, nil)
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	cc := config.CallContext{}
	_, err = r.Call(ctx, "combo", "betaEcho", cc, map[string]any{"q": "x"})
	if err == nil {
		t.Fatal("a mount must be refused while the namespace's required server waits on a secret")
	}
	if !strings.Contains(err.Error(), "AUTH_TOKEN") && !strings.Contains(err.Error(), "auth") {
		t.Errorf("the refusal must name the blocked dependency or its secret: %v", err)
	}
	// Control: the same tool called on its home server directly
	// is unaffected by the namespace's dependency.
	if _, err := r.Call(ctx, "beta", "echo", cc, map[string]any{"q": "x"}); err != nil {
		t.Fatalf("the home server itself must stay callable: %v", err)
	}
}
