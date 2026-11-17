package e2e_test

import (
	"bufio"
	"context"
	"encoding/json"
	"os/exec"
	"strings"
	"testing"
	"time"
)

// TestCoverageCLIArtifacts exercises the artifacts command (the
// artifacts_list operation, GET /v1/artifacts) through the
// coverage-recording env runner: with nothing produced yet, the
// listing is a well-formed empty set. Burn-down #8464 (the #1978
// feature-coverage guard flagged cli artifacts as exercised by no
// integration test).
func TestCoverageCLIArtifacts(t *testing.T) {
	e := newEnv(t, oneServer)
	out := e.run("artifacts", "list", "--json")
	var listing struct {
		Artifacts []json.RawMessage `json:"artifacts"`
	}
	if err := json.Unmarshal([]byte(out), &listing); err != nil {
		t.Fatalf("artifacts list: not the listing shape: %v\n%s", err, out)
	}
	if listing.Artifacts == nil {
		t.Fatalf("artifacts list: no artifacts field in the answer:\n%s", out)
	}
	if len(listing.Artifacts) != 0 {
		t.Fatalf("artifacts list: a fresh env has produced nothing, got %d entries:\n%s", len(listing.Artifacts), out)
	}
}

// TestCoverageCLIComplete exercises the complete command (POST
// /v1/complete). The fake demo server does not declare the
// completions capability, so per the operation's contract it is not
// asked: the answer comes from the cached name set and says so in
// its source field. Burn-down #8464.
func TestCoverageCLIComplete(t *testing.T) {
	e := newEnv(t, oneServer)
	e.run("ls") // starts the daemon and populates the prompt-name cache
	out := e.run("complete",
		"--server", "demo",
		"--ref", `{"type":"ref/prompt","name":"greet"}`,
		"--argument", `{"name":"who","value":""}`,
		"--json")
	var answer struct {
		Source     string `json:"source"`
		Upstream   bool   `json:"upstream"`
		Completion struct {
			Values []string `json:"values"`
			Total  int      `json:"total"`
		} `json:"completion"`
	}
	if err := json.Unmarshal([]byte(out), &answer); err != nil {
		t.Fatalf("complete: not the completion shape: %v\n%s", err, out)
	}
	if answer.Upstream {
		t.Fatalf("complete: a server without the completions capability must not be asked upstream:\n%s", out)
	}
	if answer.Source != "cache" {
		t.Fatalf("complete: want the cached answer (source=cache), got source=%q:\n%s", answer.Source, out)
	}
	if len(answer.Completion.Values) == 0 {
		t.Fatalf("complete: the cached name set is empty:\n%s", out)
	}
}

// TestCoverageCLIEvents exercises the events command (GET
// /v1/events, the one streaming operation). A stream never
// completes, so the coverage recorder — which records on successful
// completion — can never see it driven through e.run; like
// TestEventsStreamAsNDJSON, this drives the real command as a raw
// subprocess, asserts on the replayed stream, and only then calls
// e.recordCLI with the driven arguments, exactly what
// tryWithCoverage calls after a successful drive. The row is
// recorded because the command was genuinely exercised, never
// instead of exercising it. Burn-down #8464.
func TestCoverageCLIEvents(t *testing.T) {
	e := newEnv(t, oneServer)
	e.run("ls")
	args := []string{"events", "--kinds", "server", "--since", "0"}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, e.mcpx, args...)
	cmd.Dir, cmd.Env = e.dir, e.envVars
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	defer func() { _ = cmd.Process.Kill(); _ = cmd.Wait() }()

	lines := make(chan string, 16)
	go func() {
		sc := bufio.NewScanner(stdout)
		for sc.Scan() {
			lines <- sc.Text()
		}
		close(lines)
	}()
	// Keep producing server events until one arrives: the subscription
	// may not be established by the time the first one fires.
	tick := time.NewTicker(500 * time.Millisecond)
	defer tick.Stop()
	for {
		select {
		case line, ok := <-lines:
			if !ok {
				t.Fatal("the stream ended without an event")
			}
			var ev struct {
				Kind string `json:"kind"`
			}
			if err := json.Unmarshal([]byte(line), &ev); err != nil {
				t.Fatalf("not one JSON document a line: %q: %v", line, err)
			}
			if !strings.HasPrefix(ev.Kind, "server.") {
				t.Fatalf("--kinds server let through %q", ev.Kind)
			}
			e.recordCLI(args)
			return
		case <-tick.C:
			_, _ = e.try("restart", "demo")
		case <-ctx.Done():
			t.Fatalf("no event within the deadline: %v", ctx.Err())
		}
	}
}

// TestCoverageCLITool exercises the tool command (tool_invoke, POST
// /v1/tools/{tool}) with one of mcpx's own read-only MCP tools:
// mcpx_health must report the daemon healthy. Burn-down #8464.
func TestCoverageCLITool(t *testing.T) {
	e := newEnv(t, oneServer)
	out := e.run("tool", "invoke", "--tool", "mcpx_health", "--arguments", "{}")
	if !strings.Contains(out, `"status": "ok"`) {
		t.Fatalf("tool invoke mcpx_health: the daemon did not report itself healthy:\n%s", out)
	}
}
