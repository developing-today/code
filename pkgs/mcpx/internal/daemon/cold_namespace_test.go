package daemon

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"log"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/codegen"
	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// Oracle: readiness changes generated members, never namespace presence.
func TestColdNamespaceGeneration1868(t *testing.T) {
	r := scriptRegistry(t, map[string]*config.Server{"cold": {Name: "cold", Command: "must-not-start"}, "warm": {Name: "warm", Command: "must-not-start-warm"}})
	t.Cleanup(r.Close)
	warm, _ := r.Pool("warm")
	warm.SetSchemas(nil, nil, nil, "", time.Now(), time.Time{}, false)
	nss, err := r.CodegenNamespaces(nil, config.Profile{})
	if err != nil {
		t.Fatal(err)
	}
	if len(nss) != 2 {
		t.Fatalf("namespaces=%+v", nss)
	}
	for _, lang := range []string{"typescript", "python"} {
		l, _ := codegen.Lookup(lang)
		module := l.EmitModule(nss, "http://127.0.0.1:1", "test")
		if !strings.Contains(module, "lazyLanguage") {
			t.Errorf("%s cold namespace lacks lazy transport", lang)
		}
	}
	if _, err := r.CodegenNamespaces([]string{"cold.unknown_yet"}, config.Profile{}); err != nil {
		t.Fatalf("cold selected member refused before schema exists: %v", err)
	}
	if _, err := r.CodegenNamespaces([]string{"warm.unknown_yet"}, config.Profile{}); err == nil {
		t.Fatal("warm empty namespace must reject missing member")
	}
}

func TestGeneratedColdCall1868(t *testing.T) {
	for _, language := range []string{"typescript", "python"} {
		t.Run(language, func(t *testing.T) {
			upstream := newEchoUpstream(t, "fixture", []mcpclient.Tool{{Name: "odd-name", InputSchema: json.RawMessage(`{"type":"object"}`)}})
			r := scriptRegistry(t, map[string]*config.Server{"cold": {Name: "cold", URL: upstream.URL}})
			t.Cleanup(r.Close)
			raw, err := r.CallGenerated(context.Background(), "", language, "cold", "odd_name", config.CallContext{SessionID: "isolated"}, map[string]any{"q": "yes"})
			if err != nil {
				t.Fatal(err)
			}
			if !strings.Contains(string(raw), "odd-name") {
				t.Fatalf("wrong routed tool: %s", raw)
			}
			raw, err = r.CallGenerated(context.Background(), "report-fixture", language, "cold", "odd_name", config.CallContext{SessionID: "isolated"}, map[string]any{})
			if err != nil {
				t.Fatal(err)
			}
		})
	}
}

func TestGeneratedColdRefusals1868(t *testing.T) {
	for _, tc := range []struct {
		name     string
		tools    []mcpclient.Tool
		exclude  []string
		property string
	}{
		{"collision", []mcpclient.Tool{{Name: "foo-bar"}, {Name: "foo_bar"}}, nil, "foo_bar"},
		{"hidden", []mcpclient.Tool{{Name: "secret"}}, []string{"secret"}, "secret"},
		{"empty", nil, nil, "missing"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			u := newEchoUpstream(t, "fixture", tc.tools)
			r := scriptRegistry(t, map[string]*config.Server{"cold": {Name: "cold", URL: u.URL, Mcpx: &config.Extras{ExcludeTools: tc.exclude}}})
			t.Cleanup(r.Close)
			if _, err := r.CallGenerated(context.Background(), "", "typescript", "cold", tc.property, config.CallContext{}, nil); err == nil {
				t.Fatal("must refuse")
			}
		})
	}
	r := scriptRegistry(t, map[string]*config.Server{"cold": {Name: "cold", Command: "must-not-start"}})
	t.Cleanup(r.Close)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := r.CallGenerated(ctx, "", "typescript", "cold", "unknown", config.CallContext{}, nil); !errors.Is(err, context.Canceled) {
		t.Fatalf("cancel: %v", err)
	}
	if _, err := r.CallGenerated(context.Background(), "", "unknown-language", "cold", "unknown", config.CallContext{}, nil); err == nil {
		t.Fatal("invalid language")
	}
	if _, err := r.CallGenerated(context.Background(), "", "typescript", "absent", "unknown", config.CallContext{}, nil); err == nil {
		t.Fatal("unconfigured")
	}
	if _, err := r.CallGenerated(context.Background(), "", "typescript", "cold", "unknown", config.CallContext{}, nil); err == nil {
		t.Fatal("refresh failure")
	}
}

func TestGeneratedColdVirtual1868(t *testing.T) {
	r, _, _ := virtualRegistry(t)
	t.Cleanup(r.Close)
	raw, err := r.CallGenerated(context.Background(), "", "typescript", "combo", "alphaEcho", config.CallContext{}, map[string]any{"q": "x"})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "alpha") {
		t.Fatalf("wrong mount: %s", raw)
	}
	_, _, at := r.pools["beta"].CachedSchemas()
	if !at.IsZero() {
		t.Fatal("unrelated cold mount was refreshed")
	}
}

func TestGeneratedColdMetadata1868(t *testing.T) {
	off := false
	r := scriptRegistry(t, map[string]*config.Server{
		"cold":   {Name: "cold", Command: "must-not-start"},
		"warm":   {Name: "warm", Command: "must-not-start-warm"},
		"hidden": {Name: "hidden", Command: "must-not-start-hidden", Mcpx: &config.Extras{Default: &off}},
	})
	t.Cleanup(r.Close)
	r.pools["warm"].SetSchemas(nil, nil, nil, "", time.Now(), time.Time{}, false)
	ns, err := r.CodegenNamespaces(nil, config.Profile{})
	if err != nil {
		t.Fatal(err)
	}
	if len(ns) != 2 || !ns[0].Cold || ns[1].Cold {
		t.Fatalf("readiness/profile: %+v", ns)
	}
	selected, err := r.CodegenNamespaces([]string{"cold.some_tool"}, config.Profile{})
	if err != nil || len(selected) != 1 || len(selected[0].LazyTools) != 1 {
		t.Fatalf("selection: %+v %v", selected, err)
	}
}

func TestGeneratedColdScoped1868(t *testing.T) {
	u := newEchoUpstream(t, "scoped", []mcpclient.Tool{echoTool()})
	r := scriptRegistry(t, map[string]*config.Server{"scoped": {Name: "scoped", URL: u.URL, Env: map[string]string{"SCOPE_REQUIRED": "${mcpx.cwd}"}, Mcpx: &config.Extras{Scope: config.ScopeCwd}}})
	t.Cleanup(r.Close)
	if !r.pools["scoped"].CallerScoped() {
		t.Fatal("fixture must require caller scope")
	}
	for _, cwd := range []string{t.TempDir(), t.TempDir()} {
		if _, err := r.CallGenerated(context.Background(), "", "python", "scoped", "echo", config.CallContext{Cwd: cwd}, map[string]any{}); err != nil {
			t.Fatal(err)
		}
	}
	status := r.pools["scoped"].Status()
	if len(status.Instances) != 2 {
		t.Fatalf("caller instances: %+v", status)
	}
	for _, in := range status.Instances {
		if !strings.HasPrefix(in.Key, "cwd:") {
			t.Fatalf("synthetic or wrong scope: %+v", in)
		}
	}
}

func TestGeneratedColdSnapshotPolicy1868(t *testing.T) {
	p := writePolicyPool(t, "", `{"readOnlyHint":true}`)
	r := writePolicyRegistry("deny")
	snapshot := []mcpclient.Tool{{Name: "wipe", Annotations: json.RawMessage(`{"readOnlyHint":false}`)}}
	ctx := context.WithValue(context.Background(), generatedSchemasKey{}, generatedSchemas{p: snapshot})
	if _, err := r.applyWritePolicy(ctx, p, "demo", "wipe", config.CallContext{}, nil); err == nil {
		t.Fatal("another caller's read-only cache bypassed scoped policy")
	}
	if toolReadOnlyFor(ctx, p, "wipe") {
		t.Fatal("another caller's read-only cache bypassed write ledger")
	}
	if !toolReadOnly(p, "wipe") {
		t.Fatal("ordinary cache semantics changed")
	}
}

func TestGeneratedColdSelector1868(t *testing.T) {
	u := newEchoUpstream(t, "fixture", []mcpclient.Tool{{Name: "foo-bar"}, {Name: "foo_bar"}})
	r := scriptRegistry(t, map[string]*config.Server{"cold": {Name: "cold", URL: u.URL}})
	t.Cleanup(r.Close)
	for _, lang := range []string{"typescript", "python"} {
		raw, err := r.CallGenerated(context.Background(), "", lang, "cold", "foo_bar", config.CallContext{}, nil, "foo-bar")
		if err != nil || !strings.Contains(string(raw), "foo-bar") {
			t.Fatalf("exact selected raw name: %s %v", raw, err)
		}
		if _, err := r.CallGenerated(context.Background(), "", lang, "cold", "foo_bar", config.CallContext{}, nil, "foo_bar"); err == nil {
			t.Fatal("normalized selector collision must refuse")
		}
	}
}

func TestGeneratedColdInflight1868(t *testing.T) {
	for _, change := range []string{"cancel", "reload"} {
		t.Run(change, func(t *testing.T) {
			u := newEchoUpstream(t, "fixture", []mcpclient.Tool{echoTool()})
			original := u.Config.Handler
			started := make(chan struct{}, 1)
			release := make(chan struct{})
			var calls atomic.Int32
			u.Config.Handler = http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
				data, _ := io.ReadAll(req.Body)
				req.Body = io.NopCloser(bytes.NewReader(data))
				var method struct {
					Method string `json:"method"`
				}
				json.Unmarshal(data, &method)
				if method.Method == "tools/call" {
					calls.Add(1)
				}
				if method.Method == "tools/list" {
					select {
					case started <- struct{}{}:
					default:
					}
					select {
					case <-release:
					case <-req.Context().Done():
						return
					}
				}
				original.ServeHTTP(w, req)
			})
			r := scriptRegistry(t, map[string]*config.Server{"cold": {Name: "cold", URL: u.URL}})
			t.Cleanup(r.Close)
			ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
			defer cancel()
			done := make(chan error, 1)
			go func() {
				_, err := r.CallGenerated(ctx, "", "typescript", "cold", "echo", config.CallContext{}, nil)
				done <- err
			}()
			select {
			case <-started:
			case <-ctx.Done():
				close(release)
				t.Fatal("no schema request")
			}
			if change == "cancel" {
				cancel()
			} else {
				// Model the atomic view replacement performed by Reload, without starting
				// background maintenance or touching any live daemon.
				r.mu.Lock()
				cp := *r.views["cold"]
				r.views["cold"] = &cp
				r.mu.Unlock()
			}
			close(release)
			select {
			case err := <-done:
				if err == nil {
					t.Fatal("changed call must fail")
				}
				if change == "reload" && !strings.Contains(err.Error(), "configuration changed") {
					t.Fatalf("reload error: %v", err)
				}
			case <-time.After(6 * time.Second):
				t.Fatal("call did not stop")
			}
			if calls.Load() != 0 {
				t.Fatal("tool was dispatched after cancellation/reload")
			}
		})
	}
}

func TestGeneratedColdHTTP1868(t *testing.T) {
	u := newEchoUpstream(t, "fixture", []mcpclient.Tool{{Name: "odd-name"}})
	r := scriptRegistry(t, map[string]*config.Server{"cold": {Name: "cold", URL: u.URL}})
	t.Cleanup(r.Close)
	s := &Server{reg: r, logger: log.New(io.Discard, "", 0)}
	for _, body := range []string{
		`{"server":"cold","tool":"odd_name","lazyLanguage":"typescript"}`,
		`{"server":"cold","tool":"odd_name","lazyLanguage":"python"}`,
	} {
		w := httptest.NewRecorder()
		req := httptest.NewRequest("POST", "/v1/call", strings.NewReader(body))
		s.handleCall(w, req)
		if w.Code != 200 || !strings.Contains(w.Body.String(), "odd-name") {
			t.Fatalf("HTTP %d %s", w.Code, w.Body.String())
		}
	}
	for _, body := range []string{
		`{"server":"cold","tool":"odd_name","lazyLanguage":"wrong"}`,
		`{"server":"cold","tool":"odd_name","lazyLanguage":"python","task":{"ttl":1}}`,
	} {
		w := httptest.NewRecorder()
		s.handleCall(w, httptest.NewRequest("POST", "/v1/call", strings.NewReader(body)))
		if w.Code != 400 {
			t.Fatalf("invalid request status%d", w.Code)
		}
	}
	dry := dryRunServer()
	dry.callRegistry().RegisterDryRun("cold-dry")
	w := httptest.NewRecorder()
	req := httptest.NewRequest("POST", "/v1/call", strings.NewReader(`{"server":"demo","tool":"echo","lazyLanguage":"typescript"}`))
	req.Header.Set("X-Mcpx-Run", "cold-dry")
	dry.handleCall(w, req)
	if w.Code != 200 || !strings.Contains(w.Body.String(), `"dryRun":true`) {
		t.Fatalf("dryrun: %d %s", w.Code, w.Body.String())
	}
}
