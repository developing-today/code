package e2e_test

// Burn-down exercises for the inspection-group CLI commands (#8463):
// dashboard, hook, interactions and logs each get one real exercise
// through the harness against a live daemon. A successful run is what
// records the feature's coverage-ledger row, which is what lets its
// line leave feature_coverage_exemptions.txt. explore, the fifth
// command in the group, needs a terminal and lives in
// cli_inspection_explore_linux_test.go.

import (
	"io"
	"net/http"
	"strings"
	"testing"
)

func TestDashboardPrintsItsLiveURL(t *testing.T) {
	e := newEnv(t, oneServer)
	out := e.run("dashboard")
	if !strings.Contains(out, "mcpx dashboard running at ") {
		t.Fatalf("dashboard did not report where it is running:\n%s", out)
	}
	if !strings.Contains(out, "/dashboard") {
		t.Fatalf("the dashboard URL does not name the dashboard path:\n%s", out)
	}
}

func TestHookLsListsDeclaredHooks(t *testing.T) {
	e := newEnv(t, oneServer)
	if out := e.run("hook", "ls"); !strings.Contains(out, "No hooks declared") {
		t.Fatalf("an env with no hooks setting should say so:\n%s", out)
	}

	declared := newEnv(t, `{
  "mcpServers": {
    "demo": { "command": "FAKE", "mcpx": { "sharing": "shared", "scope": "global", "description": "a fake server" } }
  },
  "hooks": [ { "name": "page", "on": "server.stopped", "run": ["echo", "done"] } ]
}`)
	out := declared.run("hook", "ls")
	for _, want := range []string{"page", "server.stopped", "echo done"} {
		if !strings.Contains(out, want) {
			t.Errorf("hook ls does not show %q:\n%s", want, out)
		}
	}
}

func TestInteractionsListsSearchesAndShowsAProxyTurn(t *testing.T) {
	e := newEnv(t, oneServer)
	if out := e.run("interactions"); !strings.Contains(out, "No matching interactions found.") {
		t.Fatalf("a fresh daemon should report an empty history:\n%s", out)
	}

	// Interactions are recorded by the daemon's proxy surface: a chat
	// completion with no upstream configured is answered locally and
	// leaves a trace behind. Drive one through the daemon socket,
	// then read it back through every interactions surface: list,
	// search and --show.
	req, err := http.NewRequest(http.MethodPost, "http://mcpx/v1/chat/completions",
		strings.NewReader(`{"messages":[{"role":"user","content":"inspectionprobe question"}]}`))
	if err != nil {
		t.Fatal(err)
	}
	resp, err := e.socketClient(t).Do(req)
	if err != nil {
		t.Fatal(err)
	}
	body, _ := io.ReadAll(resp.Body)
	_ = resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("the proxy turn failed: status %d: %s", resp.StatusCode, body)
	}

	list := e.run("interactions")
	if !strings.Contains(list, "TRACE ID") || !strings.Contains(list, "inspectionprobe") {
		t.Fatalf("the recorded turn is not in the interactions list:\n%s", list)
	}
	var trace string
	for _, l := range strings.Split(list, "\n") {
		if strings.Contains(l, "inspectionprobe") {
			trace = strings.Fields(l)[0]
			break
		}
	}
	if trace == "" {
		t.Fatalf("no trace id in the interactions list:\n%s", list)
	}

	shown := e.run("interactions", "--show", trace)
	if !strings.Contains(shown, "Trace ID:") || !strings.Contains(shown, "inspectionprobe") {
		t.Fatalf("--show does not render the recorded turn:\n%s", shown)
	}
	if out := e.run("interactions", "--q", "inspectionprobe"); !strings.Contains(out, trace) {
		t.Fatalf("search should find the recorded turn:\n%s", out)
	}
	if out := e.run("interactions", "--q", "no-such-input-zzz"); !strings.Contains(out, "No matching interactions found.") {
		t.Fatalf("a search with no hits should say so:\n%s", out)
	}
}

// The log command's canonical spelling is covered by logs_test.go;
// the dispatcher's "logs" alias key is a registry feature of its own,
// and this is its exercise — the same surfaces, spelled "logs".
func TestLogsAliasQueriesTheLog(t *testing.T) {
	e := newEnv(t, oneServer)
	e.run("call", "demo.echo", `{"message":"x"}`)

	out := e.run("logs", "sql", "SELECT count(*) AS n FROM records")
	if !strings.Contains(out, "n") {
		t.Fatalf("logs sql did not print a table:\n%s", out)
	}

	slowest := e.run("stats", "slowest")
	var trace string
	for _, l := range strings.Split(slowest, "\n") {
		if !strings.Contains(l, "echo") {
			continue
		}
		f := strings.Fields(l)
		trace = f[len(f)-1]
	}
	if !strings.HasPrefix(trace, "cal-") {
		t.Fatalf("no call trace id in:\n%s", slowest)
	}
	chain := e.run("logs", "--chain", trace)
	for _, want := range []string{"dmn-", "srv-", "cal-", "daemon starting"} {
		if !strings.Contains(chain, want) {
			t.Fatalf("the chain does not reach %q:\n%s", want, chain)
		}
	}
}

// TestWantRecordsAndListsACapabilityRequest exercises the want command:
// a request whose first word is not a verb is recorded as typed, and the
// list shows it back.
func TestWantRecordsAndListsACapabilityRequest(t *testing.T) {
	e := newEnv(t, oneServer)
	out := e.run("want", "calendar", "mcp")
	if !strings.Contains(out, `Recorded capability request "calendar mcp"`) {
		t.Fatalf("want did not record the request:\n%s", out)
	}
	out = e.run("want", "list", "--kind", "request")
	if !strings.Contains(out, "calendar mcp") {
		t.Fatalf("want list does not show the recorded request:\n%s", out)
	}
}
