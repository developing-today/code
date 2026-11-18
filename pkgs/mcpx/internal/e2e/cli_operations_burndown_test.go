package e2e_test

// Burn-down exercises for the operations-group CLI commands (#8464):
// artifacts, complete, events and tool each get one real exercise
// through the harness against a live daemon. A successful run is
// what records the feature's coverage-ledger row, which is what lets
// its line leave feature_coverage_exemptions.txt. All four are
// generated commands (opcmd.go): the exercises drive them the way a
// shell user does.

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

// The artifact command's canonical spelling is covered by
// cliops_test.go; the dispatcher's "artifacts" alias key is a
// registry feature of its own, and this is its exercise — the same
// round trip, spelled "artifacts".
func TestArtifactsAliasRoundTripsAFile(t *testing.T) {
	e := newEnv(t, oneServer)
	src := filepath.Join(e.dir, "note.txt")
	if err := os.WriteFile(src, []byte("through the alias"), 0o644); err != nil {
		t.Fatal(err)
	}

	var put struct {
		ID   string `json:"id"`
		Size int    `json:"size"`
	}
	if err := json.Unmarshal([]byte(e.run("--json", "artifacts", "put", "note.txt", "@"+src)), &put); err != nil {
		t.Fatal(err)
	}
	if put.ID == "" || put.Size != len("through the alias") {
		t.Fatalf("put returned %+v, want an id and size %d", put, len("through the alias"))
	}

	listed := e.run("artifacts", "ls")
	if !strings.Contains(listed, put.ID) || !strings.Contains(listed, "note.txt") {
		t.Fatalf("artifacts ls does not show the artifact:\n%s", listed)
	}
	if got := e.run("artifacts", "get", put.ID); strings.TrimSpace(got) != "through the alias" {
		t.Fatalf("artifacts get printed %q", got)
	}

	e.run("artifacts", "rm", put.ID)
	if listed := e.run("artifacts", "list"); strings.Contains(listed, put.ID) {
		t.Fatalf("artifacts rm left it listed:\n%s", listed)
	}
}

func TestCompleteFallsBackToTheCacheForAServerWithoutTheCapability(t *testing.T) {
	e := newEnv(t, oneServer)
	out := e.run("--json", "complete", "--server", "demo",
		"--ref", `{"type":"ref/prompt","name":"summarise"}`,
		"--argument", `{"name":"style","value":"in"}`)

	// The fake server never declared the completions capability, so
	// the daemon does not ask it: the answer comes from the cache and
	// says so, with an empty value list rather than an error.
	var doc struct {
		Source     string `json:"source"`
		Because    string `json:"because"`
		Upstream   bool   `json:"upstream"`
		Completion struct {
			Values []string `json:"values"`
			Total  int      `json:"total"`
		} `json:"completion"`
	}
	if err := json.Unmarshal([]byte(out), &doc); err != nil {
		t.Fatalf("complete did not answer with the completion document: %v\n%s", err, out)
	}
	if doc.Source != "cache" || doc.Upstream {
		t.Fatalf("complete should have been answered from the cache: %+v", doc)
	}
	if !strings.Contains(doc.Because, "completions capability") {
		t.Fatalf("the answer should say why the server was not asked: %+v", doc)
	}
	if doc.Completion.Values == nil || doc.Completion.Total != 0 {
		t.Fatalf("an unimplemented completion answers with an empty value list: %+v", doc.Completion)
	}
}

func TestToolInvokeRunsOneOfMcpxsOwnTools(t *testing.T) {
	e := newEnv(t, oneServer)
	out := e.run("--json", "tool", "invoke", "mcpx_namespaces")
	var doc struct {
		OK     bool   `json:"ok"`
		Result string `json:"result"`
	}
	if err := json.Unmarshal([]byte(out), &doc); err != nil {
		t.Fatalf("tool invoke did not answer with the tool result: %v\n%s", err, out)
	}
	if !doc.OK {
		t.Fatalf("tool invoke reported not-ok:\n%s", out)
	}
	if !strings.Contains(doc.Result, "demo") || !strings.Contains(doc.Result, "ready") {
		t.Fatalf("mcpx_namespaces did not list the ready demo namespace:\n%s", doc.Result)
	}
}

// lockedBuffer is an io.Writer safe for the stream runner's reader
// goroutine pattern: the test polls what the command has printed so
// far while the command is still running.
type lockedBuffer struct {
	mu  sync.Mutex
	buf bytes.Buffer
}

func (w *lockedBuffer) Write(p []byte) (int, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	return w.buf.Write(p)
}

func (w *lockedBuffer) String() string {
	w.mu.Lock()
	defer w.mu.Unlock()
	return w.buf.String()
}

// sigintRunner runs the env's binary and ends it the way a person
// ends a stream that has no end of its own: Ctrl-C, once it has
// printed something. The CLI treats that context cancellation as a
// clean finish, which is what lets the run record coverage.
type sigintRunner struct{}

func (sigintRunner) name() string       { return "sigint" }
func (sigintRunner) needsProcess() bool { return true }

func (sigintRunner) run(ctx context.Context, e *env, _ io.Reader, args ...string) (string, error) {
	cmd := exec.Command(e.mcpx, args...)
	cmd.Dir = e.dir
	cmd.Env = e.envVars
	out := &lockedBuffer{}
	cmd.Stdout = out
	cmd.Stderr = out
	if err := cmd.Start(); err != nil {
		return "", err
	}
	deadline := time.Now().Add(30 * time.Second)
	for time.Now().Before(deadline) {
		if strings.Contains(out.String(), "\n") {
			break
		}
		time.Sleep(100 * time.Millisecond)
	}
	// Let the frames already on their way land before interrupting.
	time.Sleep(500 * time.Millisecond)
	_ = cmd.Process.Signal(os.Interrupt)
	err := cmd.Wait()
	return out.String(), err
}

func TestEventsReplaysTheBusAndEndsOnInterrupt(t *testing.T) {
	e := newEnv(t, oneServer)
	// Give the bus something to replay: starting the server and
	// making a call are both events.
	e.run("call", "demo.echo", `{"message":"x"}`)

	e.runner = sigintRunner{}
	out := e.run("events", "--since", "0")
	// The env's cleanup stops the daemon through the env's runner;
	// give it back the ordinary one first.
	e.runner = binaryRunner{}

	for _, want := range []string{`"kind":"server.started"`, `"kind":"call.finished"`, `"tool":"echo"`} {
		if !strings.Contains(out, want) {
			t.Fatalf("the replayed stream does not contain %s:\n%s", want, out)
		}
	}
}
