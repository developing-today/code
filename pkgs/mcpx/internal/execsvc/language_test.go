package execsvc

import (
	"context"
	"encoding/json"
	"os/exec"
	"strings"
	"testing"
	"time"
)

// TestExecOptionsLanguageWireName pins the /v1/exec spelling of the
// explicit script-language selection (#1917 remainder): the JSON field
// is "language", matching script.language and --language.
func TestExecOptionsLanguageWireName(t *testing.T) {
	var opts Options
	if err := json.Unmarshal([]byte(`{"language":"python"}`), &opts); err != nil {
		t.Fatal(err)
	}
	if opts.Language != "python" {
		t.Fatalf("Options.Language = %q, want python", opts.Language)
	}
	out, err := json.Marshal(Options{Language: "python"})
	if err != nil {
		t.Fatal(err)
	}
	if string(out) != `{"language":"python"}` {
		t.Fatalf("marshalled options = %s, want the language field only", out)
	}
}

// fakeLanguageSource is a Source whose Python client is a marker
// module: a run that installs it proves the service selected the
// Python client for that run.
type fakeLanguageSource struct{}

func (fakeLanguageSource) ClientModule(context.Context, []string) (string, error) {
	return "export {}", nil
}

func (fakeLanguageSource) Globals(context.Context, []string) (string, error) {
	return "", nil
}

func (fakeLanguageSource) Namespaces(context.Context, []string) ([]string, error) {
	return nil, nil
}

func (fakeLanguageSource) ClientModuleFor(_ context.Context, language string, _ []string) (string, string, error) {
	return "# fake " + language + " client\n", "", nil
}

// TestConfiguredLanguageAppliesToRuns is the Codex finding on PR #7818:
// the daemon's configured script.language (Limits.Language) must reach
// a /v1/exec run that names no language of its own, and a request's
// explicit language must override the configured default. The oracle
// is a snippet that is Python and not TypeScript: it only runs when
// the language selection actually selected Python.
func TestConfiguredLanguageAppliesToRuns(t *testing.T) {
	if _, err := exec.LookPath("python3"); err != nil {
		t.Skip("no python3 on PATH:", err)
	}
	run := func(t *testing.T, lim, opt string) string {
		t.Helper()
		limits := DefaultLimits()
		limits.Timeout = 30 * time.Second
		limits.Language = lim
		svc := &Service{Src: fakeLanguageSource{}, Limits: limits, Local: true, WorkDir: t.TempDir()}
		res, err := svc.Run(context.Background(), Request{
			Source: "print('py-ok')\n",
			Opts:   Options{Language: opt},
		}, func(Frame) error { return nil })
		if err != nil {
			t.Fatalf("run (limits %q, opts %q): %v", lim, opt, err)
		}
		if res.Error != "" || res.ExitCode != 0 {
			t.Fatalf("run (limits %q, opts %q) failed: %+v", lim, opt, res)
		}
		return res.Stdout
	}
	if out := run(t, "python", ""); !strings.Contains(out, "py-ok") {
		t.Fatalf("configured python did not run the snippet as Python: stdout %q", out)
	}
	if out := run(t, "typescript", "python"); !strings.Contains(out, "py-ok") {
		t.Fatalf("explicit python did not override the configured default: stdout %q", out)
	}
}
