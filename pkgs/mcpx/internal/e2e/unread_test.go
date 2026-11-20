package e2e_test

import (
	"bytes"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/creack/pty"
)

// These are the settings that were declared and never read. Each test here
// sets one the way a user would -- a variable or a config key, not a Go field
// -- and asserts that something observable changed. A unit test on the wiring
// would have passed for most of these before they were wired, because the
// wiring was the part that did not exist.

// plainServer has no mcpx block, so the pool defaults are what decide it.
// oneServer states sharing and scope per server, which would mask the very
// layer under test.
const plainServer = `{
  "mcpServers": {
    "demo": { "command": "FAKE" }
  }
}`

type configRow struct {
	Name    string `json:"name"`
	Sharing string `json:"sharing"`
	Scope   string `json:"scope"`
	Max     int    `json:"max"`
}

func (e *env) configRows(t *testing.T, extraEnv ...string) []configRow {
	t.Helper()
	saved := e.envVars
	e.envVars = append(append([]string{}, e.envVars...), extraEnv...)
	defer func() { e.envVars = saved }()
	out := e.run("--json", "config")
	var doc struct {
		Servers []configRow `json:"servers"`
	}
	if err := json.Unmarshal([]byte(out), &doc); err != nil {
		t.Fatalf("config --json should be a document: %v\n%s", err, out)
	}
	return doc.Servers
}

func TestPoolDefaultsAreReachableFromTheEnvironment(t *testing.T) {
	e := newEnv(t, plainServer)

	base := e.configRows(t)
	if len(base) != 1 || base[0].Sharing != "shared" {
		t.Fatalf("expected one shared server by default, got %+v", base)
	}

	// Scope stays global so that max is not clamped to one by the
	// single-key rule in config.Resolve.
	got := e.configRows(t,
		"MCPX_POOL_SHARING=exclusive",
		"MCPX_POOL_SCOPE=session",
		"MCPX_POOL_MAX=7")
	if len(got) != 1 {
		t.Fatalf("expected one server, got %+v", got)
	}
	if got[0].Sharing != "exclusive" || got[0].Scope != "session" || got[0].Max != 7 {
		t.Fatalf("the pool variables did not reach the resolved server: %+v", got[0])
	}
}

func TestPerServerPoolSettingsStillBeatTheEnvironment(t *testing.T) {
	// The fold writes the layer a config file's `pool` block writes, which
	// is beneath a server's own mcpx block. If it wrote any higher, a
	// variable meant as a default would override a deliberate per-server
	// choice, which is the opposite of what a default is.
	e := newEnv(t, oneServer)
	got := e.configRows(t, "MCPX_POOL_SHARING=exclusive")
	if len(got) != 1 || got[0].Sharing != "shared" {
		t.Fatalf("mcpx.sharing on the server should win: %+v", got)
	}
}

func TestJSONIsReachableAfterTheSubcommand(t *testing.T) {
	// --json was consumed by main before the subcommand, so `mcpx ls --json`
	// set output.json in the registry and printed a table anyway.
	e := newEnv(t, oneServer)
	out := e.run("ls", "--json")
	var any []map[string]any
	if err := json.Unmarshal([]byte(out), &any); err != nil {
		t.Fatalf("`ls --json` should print one document: %v\n%s", err, out)
	}
}

func TestOutputJSONIsReachableFromTheEnvironment(t *testing.T) {
	e := newEnv(t, oneServer)
	saved := e.envVars
	e.envVars = append(append([]string{}, saved...), "MCPX_OUTPUT_JSON=true")
	defer func() { e.envVars = saved }()
	out := e.run("ls")
	var any []map[string]any
	if err := json.Unmarshal([]byte(out), &any); err != nil {
		t.Fatalf("MCPX_OUTPUT_JSON should produce a document: %v\n%s", err, out)
	}
}

func TestOutputColorIsReachableFromTheEnvironment(t *testing.T) {
	e := newEnv(t, oneServer)
	// The app's deferred cleanup owns an inline daemon; a detached autostart
	// daemon would outlive this test process.
	e.envVars = append(e.envVars, "MCPX_DAEMON_INLINE=always")

	// Give the TUI contradictory ambient hints. Its configured mode must win:
	// always removes NO_COLOR, while never removes CLICOLOR_FORCE. A PTY makes
	// the ordinary interactive surface available to the child process.
	colorOutput := func(mode string) []byte {
		t.Helper()
		cmd := exec.Command(e.mcpx, "tui")
		cmd.Dir = e.dir
		cmd.Env = append(append([]string{}, e.envVars...),
			"TERM=xterm-256color",
			"MCPX_OUTPUT_COLOR="+mode,
			"NO_COLOR=1",
			"CLICOLOR_FORCE=1",
		)
		master, err := pty.StartWithSize(cmd, &pty.Winsize{Rows: 24, Cols: 80})
		if err != nil {
			t.Fatalf("start mcpx tui in a PTY: %v", err)
		}
		defer master.Close()

		// Bubble Tea switches the PTY to raw mode during startup. Repeating q
		// ensures one reaches the TUI even if the first write lands before that
		// switch; the test is bounded so a broken TUI cannot hang the suite.
		stopInput := make(chan struct{})
		defer close(stopInput)
		go func() {
			ticker := time.NewTicker(100 * time.Millisecond)
			defer ticker.Stop()
			for {
				select {
				case <-stopInput:
					return
				case <-ticker.C:
					_, _ = master.Write([]byte("q\r"))
				}
			}
		}()

		output := make(chan []byte, 1)
		go func() {
			var captured []byte
			buf := make([]byte, 256)
			answeredCursor := false
			for {
				n, readErr := master.Read(buf)
				if n > 0 {
					captured = append(captured, buf[:n]...)
					if !answeredCursor && bytes.Contains(captured, []byte("\x1b[6n")) {
						// Bubble Tea asks the terminal for its cursor position while
						// initializing. A PTY has no terminal emulator to answer.
						_, _ = master.Write([]byte("\x1b[1;1R"))
						answeredCursor = true
					}
				}
				if readErr != nil {
					break
				}
			}
			output <- captured
		}()
		done := make(chan error, 1)
		go func() { done <- cmd.Wait() }()
		select {
		case err := <-done:
			if err != nil {
				captured := <-output
				t.Fatalf("mcpx tui (output.color=%s): %v\n%s", mode, err, captured)
			}
		case <-time.After(10 * time.Second):
			_ = cmd.Process.Kill()
			<-done
			captured := <-output
			t.Fatalf("mcpx tui (output.color=%s) did not exit after q:\n%s", mode, captured)
		}
		select {
		case captured := <-output:
			return captured
		case <-time.After(time.Second):
			t.Fatal("timed out collecting mcpx tui output")
			return nil
		}
	}

	forced := colorOutput("always")
	if !strings.Contains(string(forced), "\x1b[38;") && !strings.Contains(string(forced), "\x1b[1;") {
		t.Fatalf("MCPX_OUTPUT_COLOR=always should render ANSI foreground colors in a PTY:\n%s", forced)
	}
	plain := colorOutput("never")
	if strings.Contains(string(plain), "\x1b[38;") || strings.Contains(string(plain), "\x1b[1;") {
		t.Fatalf("MCPX_OUTPUT_COLOR=never should suppress ANSI foreground colors in a PTY:\n%s", plain)
	}
}

func TestScriptPrefixIsReachableFromTheEnvironment(t *testing.T) {
	e := newEnv(t, oneServer)
	script := filepath.Join(e.dir, "s.ts")
	if err := os.WriteFile(script,
		[]byte("export default function () { return \"done\" }\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	saved := e.envVars
	e.envVars = append(append([]string{}, saved...),
		`MCPX_SCRIPT_PREFIX=console.info("the prefix ran")`)
	defer func() { e.envVars = saved }()

	out := e.run("run", script)
	if !strings.Contains(out, "the prefix ran") {
		t.Fatalf("MCPX_SCRIPT_PREFIX should reach the launcher:\n%s", out)
	}
}

func TestScriptEnvIsReachableFromTheEnvironment(t *testing.T) {
	e := newEnv(t, oneServer)
	saved := e.envVars
	e.envVars = append(append([]string{}, saved...), "MCPX_SCRIPT_ENV=FROM_SETTING=yes")
	defer func() { e.envVars = saved }()

	out := e.run("exec", `console.log(Deno.env.get("FROM_SETTING") ?? "missing")`)
	if !strings.Contains(out, "yes") {
		t.Fatalf("MCPX_SCRIPT_ENV should reach the script's environment:\n%s", out)
	}
}

// The with/without difference for script.devshell: an unknown value
// refuses the run, naming the setting, where the host default runs the
// same script fine. (A flake: value needs a real nix + flake, which the
// harness does not stage; the wrap argv itself is pinned by the runner
// unit tests.)
func TestScriptDevshellIsReachableFromTheEnvironment(t *testing.T) {
	e := newEnv(t, oneServer)
	if out := e.run("exec", `console.log("ran")`); !strings.Contains(out, "ran") {
		t.Fatalf("the host default should run the script:\n%s", out)
	}
	saved := e.envVars
	e.envVars = append(append([]string{}, saved...), "MCPX_SCRIPT_DEVSHELL=bogus")
	defer func() { e.envVars = saved }()

	out, err := e.try("exec", `console.log("ran")`)
	if err == nil {
		t.Fatalf("MCPX_SCRIPT_DEVSHELL=bogus should refuse the run:\n%s", out)
	}
	if !strings.Contains(out, "script.devshell") {
		t.Fatalf("the refusal should name script.devshell:\n%s", out)
	}
}

func TestLoggingFileOffWritesNoDurableLog(t *testing.T) {
	e := newEnv(t, oneServer)
	saved := e.envVars
	e.envVars = append(append([]string{}, saved...), "MCPX_LOGGING_FILE=false")
	defer func() { e.envVars = saved }()

	e.run("exec", `console.log("hello")`)

	logs := filepath.Join(e.dir, "state", "logs")
	entries, err := os.ReadDir(logs)
	if err != nil {
		return // no directory at all is the strongest form of the answer
	}
	for _, en := range entries {
		if strings.HasSuffix(en.Name(), ".jsonl") {
			t.Fatalf("logging.file was off and %s was written anyway", en.Name())
		}
	}
}

func TestStrictUnknownKeysRefusesATypo(t *testing.T) {
	e := newEnv(t, oneServer)
	cfg := filepath.Join(e.dir, ".mcpx.json")
	body, err := os.ReadFile(cfg)
	if err != nil {
		t.Fatal(err)
	}
	withTypo := strings.Replace(string(body), "{\n", "{\n  \"pooll\": {},\n", 1)
	if err := os.WriteFile(cfg, []byte(withTypo), 0o644); err != nil {
		t.Fatal(err)
	}

	// Off by default: a key mcpx does not claim is somebody else's section.
	if out, err := e.try("ls"); err != nil {
		t.Fatalf("an unknown key should be tolerated by default: %v\n%s", err, out)
	}

	saved := e.envVars
	e.envVars = append(append([]string{}, saved...), "MCPX_PLUMBING_STRICT_UNKNOWN_KEYS=true")
	defer func() { e.envVars = saved }()
	out, err := e.try("ls")
	if err == nil {
		t.Fatalf("strictUnknownKeys should refuse an unclaimed key:\n%s", out)
	}
	if !strings.Contains(out, "pooll") {
		t.Fatalf("the refusal should name the key:\n%s", out)
	}
}

func TestStateDirectoryIsReachableAsASetting(t *testing.T) {
	// paths.state was readable from MCPX_STATE_DIR and from nowhere else,
	// so the config key and the flag that the registry promises did nothing.
	e := newEnv(t, oneServer)
	elsewhere := filepath.Join(e.dir, "elsewhere")

	// MCPX_STATE_DIR is the same setting under its alias, and two names for
	// one setting in an environment that has no order is refused on purpose.
	saved := e.envVars
	var kept []string
	for _, kv := range saved {
		if !strings.HasPrefix(kv, "MCPX_STATE_DIR=") {
			kept = append(kept, kv)
		}
	}
	e.envVars = append(kept, "MCPX_PATHS_STATE="+elsewhere)
	defer func() { e.envVars = saved }()

	e.run("ls")
	if _, err := os.Stat(elsewhere); err != nil {
		t.Fatalf("paths.state should have decided where the daemon lives: %v", err)
	}
	// Stop the daemon this test started; the cleanup in newEnv runs without
	// the override and would look in the wrong directory for it.
	_, _ = e.try("stop")
}
