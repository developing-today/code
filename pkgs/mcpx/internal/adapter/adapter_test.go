package adapter_test

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/adapter"
)

func echoSpec() adapter.Spec {
	return adapter.Spec{
		Name: "demo", Command: "echo",
		Tools: []adapter.ToolSpec{
			{
				Name: "say", Args: []string{"said:"},
				Params: []adapter.Param{
					{Name: "text", Required: true},
					{Name: "loud", Type: "boolean", Flag: "--loud"},
					{Name: "tag", Flag: "--tag"},
					{Name: "many", Type: "array", Flag: "-m", Repeat: true},
				},
			},
		},
	}
}

func TestAPositionalParameterBecomesAnArgument(t *testing.T) {
	res, err := echoSpec().Call(context.Background(), "say", map[string]any{"text": "hello"})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(res.Stdout, "said: hello") {
		t.Errorf("got %q", res.Stdout)
	}
}

func TestABooleanIsItsPresenceNotItsValue(t *testing.T) {
	// --verbose true is wrong for almost every program ever written.
	res, _ := echoSpec().Call(context.Background(), "say",
		map[string]any{"text": "x", "loud": true})
	if !strings.Contains(res.Stdout, "--loud") {
		t.Errorf("a true boolean should appear as the flag: %q", res.Stdout)
	}
	if strings.Contains(res.Stdout, "--loud true") {
		t.Errorf("and only as the flag: %q", res.Stdout)
	}
	res, _ = echoSpec().Call(context.Background(), "say",
		map[string]any{"text": "x", "loud": false})
	if strings.Contains(res.Stdout, "--loud") {
		t.Errorf("a false boolean should not appear at all: %q", res.Stdout)
	}
}

func TestARepeatedFlagIsSentOncePerValue(t *testing.T) {
	// Which is what most programs actually accept, unlike a comma-joined one.
	res, _ := echoSpec().Call(context.Background(), "say",
		map[string]any{"text": "x", "many": []any{"a", "b"}})
	if strings.Count(res.Stdout, "-m") != 2 {
		t.Errorf("expected the flag twice: %q", res.Stdout)
	}
}

func TestAMissingRequiredParameterIsRefusedBeforeRunning(t *testing.T) {
	_, err := echoSpec().Call(context.Background(), "say", map[string]any{})
	if err == nil {
		t.Fatal("a missing required parameter should be refused")
	}
	if !strings.Contains(err.Error(), "text is required") {
		t.Errorf("the message should name it: %v", err)
	}
}

func TestAnUnknownToolNamesWhatExists(t *testing.T) {
	_, err := echoSpec().Call(context.Background(), "nope", nil)
	if err == nil || !strings.Contains(err.Error(), "say") {
		t.Errorf("the error should list the real tools: %v", err)
	}
}

func TestStdinCarriesAParameterInstead(t *testing.T) {
	// Some programs only take input that way, and everything else about them
	// is still worth exposing.
	spec := adapter.Spec{
		Name: "c", Command: "cat",
		Tools: []adapter.ToolSpec{{
			Name: "through", Stdin: "body",
			Params: []adapter.Param{{Name: "body", Required: true}},
		}},
	}
	res, err := spec.Call(context.Background(), "through", map[string]any{"body": "piped"})
	if err != nil {
		t.Fatal(err)
	}
	if strings.TrimSpace(res.Stdout) != "piped" {
		t.Errorf("got %q", res.Stdout)
	}
	if strings.Contains(res.Command, "piped") {
		t.Errorf("a stdin parameter must not also reach the command line: %q", res.Command)
	}
}

func TestANonZeroExitIsAResultNotAnError(t *testing.T) {
	// The program ran and said something; deciding that is a failure belongs
	// to whoever asked.
	spec := adapter.Spec{
		Name: "f", Command: "false",
		Tools: []adapter.ToolSpec{{Name: "fail"}},
	}
	res, err := spec.Call(context.Background(), "fail", nil)
	if err != nil {
		t.Fatalf("running it is not an error: %v", err)
	}
	if res.ExitCode == 0 {
		t.Error("the exit code should be reported")
	}
}

func TestJSONOutputIsParsedWhenAsked(t *testing.T) {
	spec := adapter.Spec{
		Name: "j", Command: "echo",
		Tools: []adapter.ToolSpec{{
			Name: "doc", JSON: true,
			Params: []adapter.Param{{Name: "body"}},
		}},
	}
	res, err := spec.Call(context.Background(), "doc", map[string]any{"body": `{"a":1}`})
	if err != nil {
		t.Fatal(err)
	}
	if len(res.JSON) == 0 {
		t.Fatalf("should have parsed: %+v", res)
	}
	var doc map[string]any
	if err := json.Unmarshal(res.JSON, &doc); err != nil || doc["a"] != 1.0 {
		t.Errorf("got %s", res.JSON)
	}
}

func TestSchemaDescribesTheParameters(t *testing.T) {
	// An adapted program should be indistinguishable from a real server to
	// anything reading schemas.
	s := echoSpec().Tools[0].Schema()
	var doc struct {
		Type       string `json:"type"`
		Required   []string
		Properties map[string]struct {
			Type string `json:"type"`
		} `json:"properties"`
	}
	if err := json.Unmarshal(s, &doc); err != nil {
		t.Fatal(err)
	}
	if doc.Type != "object" {
		t.Errorf("got %q", doc.Type)
	}
	if doc.Properties["loud"].Type != "boolean" {
		t.Errorf("types should survive: %+v", doc.Properties)
	}
}

func TestValidateCatchesContradictions(t *testing.T) {
	bad := adapter.Spec{
		Name: "x", Command: "true",
		Tools: []adapter.ToolSpec{
			{Name: "a", Stdin: "nothere", Params: []adapter.Param{
				{Name: "p", Required: true, Default: "d"},
			}},
			{Name: "a"},
		},
	}
	err := bad.Validate()
	if err == nil {
		t.Fatal("expected problems")
	}
	for _, want := range []string{"two tools named a", "not a parameter", "also has a default"} {
		if !strings.Contains(err.Error(), want) {
			t.Errorf("missing %q from:\n%v", want, err)
		}
	}
}

func TestEnumIsEnforced(t *testing.T) {
	spec := adapter.Spec{
		Name: "e", Command: "echo",
		Tools: []adapter.ToolSpec{{
			Name: "pick", Params: []adapter.Param{
				{Name: "mode", Enum: []string{"fast", "slow"}},
			},
		}},
	}
	if _, err := spec.Call(context.Background(), "pick", map[string]any{"mode": "sideways"}); err == nil {
		t.Fatal("a value outside the enum should be refused")
	}
}

func TestStdinEnumIsEnforced(t *testing.T) {
	// The stdin diversion must not skip validation: an out-of-enum value
	// for the stdin parameter is refused exactly like any other (#1648).
	spec := adapter.Spec{
		Name: "c", Command: "cat",
		Tools: []adapter.ToolSpec{{
			Name: "through", Stdin: "body",
			Params: []adapter.Param{
				{Name: "body", Required: true, Enum: []string{"fast", "slow"}},
			},
		}},
	}
	_, err := spec.Call(context.Background(), "through", map[string]any{"body": "sideways"})
	if err == nil {
		t.Fatal("an out-of-enum stdin value should be refused")
	}
	if !strings.Contains(err.Error(), `body must be one of fast, slow, got "sideways"`) {
		t.Errorf("the refusal should match the ordinary enum error: %v", err)
	}
	res, err := spec.Call(context.Background(), "through", map[string]any{"body": "fast"})
	if err != nil {
		t.Fatal(err)
	}
	if strings.TrimSpace(res.Stdout) != "fast" {
		t.Errorf("an in-enum value must still reach stdin unchanged: %q", res.Stdout)
	}
}

func TestValidateRefusesAPositionalBoolean(t *testing.T) {
	// A boolean with no flag can never take effect — the declaration is
	// wrong, and Validate says so naming the parameter (#1648).
	bad := adapter.Spec{
		Name: "x", Command: "echo",
		Tools: []adapter.ToolSpec{{
			Name: "say", Params: []adapter.Param{
				{Name: "text"},
				{Name: "loud", Type: "boolean"},
			},
		}},
	}
	err := bad.Validate()
	if err == nil {
		t.Fatal("a positional boolean should not validate")
	}
	if !strings.Contains(err.Error(), "loud") || !strings.Contains(err.Error(), "no flag") {
		t.Errorf("the refusal should name the parameter and the missing flag: %v", err)
	}

	ok := adapter.Spec{
		Name: "x", Command: "echo",
		Tools: []adapter.ToolSpec{{
			Name: "say", Stdin: "body",
			Params: []adapter.Param{
				{Name: "loud", Type: "boolean", Flag: "--loud"},
				{Name: "body", Type: "boolean"},
			},
		}},
	}
	if err := ok.Validate(); err != nil {
		t.Errorf("a flagged boolean and a boolean stdin parameter should validate: %v", err)
	}
}

func TestAnUnknownArgumentNamesIt(t *testing.T) {
	// A key naming no declared parameter is a caller mistake, not a silent
	// no-op: the error names the unknown key and what exists (#1648).
	_, err := echoSpec().Call(context.Background(), "say",
		map[string]any{"text": "hello", "tga": "important"})
	if err == nil {
		t.Fatal("an unknown argument should be refused")
	}
	for _, want := range []string{`unknown argument "tga"`, "text", "tag"} {
		if !strings.Contains(err.Error(), want) {
			t.Errorf("missing %q from:\n%v", want, err)
		}
	}
}

func TestValidateRefusesNegativeTimeouts(t *testing.T) {
	// A negative timeout passes time.ParseDuration, so without an explicit
	// refusal it validates and then every call is born expired (#1758).
	server := echoSpec()
	server.Timeout = "-5s"
	err := server.Validate()
	if err == nil {
		t.Fatal("a negative server timeout should not validate")
	}
	for _, want := range []string{"negative", `"-5s"`} {
		if !strings.Contains(err.Error(), want) {
			t.Errorf("server error should name the offending value, missing %q from:\n%v", want, err)
		}
	}

	tool := echoSpec()
	tool.Tools[0].Timeout = "-1ms"
	err = tool.Validate()
	if err == nil {
		t.Fatal("a negative tool timeout should not validate")
	}
	for _, want := range []string{"negative", `"-1ms"`} {
		if !strings.Contains(err.Error(), want) {
			t.Errorf("tool error should name the offending value, missing %q from:\n%v", want, err)
		}
	}
}

func TestZeroTimeoutIsUnset(t *testing.T) {
	// Zero is unset at both levels: a "0s" tool timeout inherits the server
	// timeout, and a "0s" server timeout means the 60s default - never an
	// already-expired context (#1758).
	spec := echoSpec()
	spec.Timeout = "0s"
	spec.Tools[0].Timeout = "0s"
	if err := spec.Validate(); err != nil {
		t.Fatalf("zero timeouts are unset and should validate: %v", err)
	}
	res, err := spec.Call(context.Background(), "say", map[string]any{"text": "hello"})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(res.Stdout, "said: hello") {
		t.Errorf("got %q", res.Stdout)
	}
}

func TestAnUnsupportedRuntimeIsRefusedWithTheList(t *testing.T) {
	spec := echoSpec()
	spec.Runtime = "ruby"
	err := spec.Validate()
	if err == nil {
		t.Fatal("ruby is not one of bash|js|ts|py")
	}
	if !strings.Contains(err.Error(), `runtime "ruby" is not supported; want bash|js|ts|py`) {
		t.Errorf("the message should name the supported set: %v", err)
	}
}

func TestSupportedRuntimesValidateWithoutAnInterpreterInstalled(t *testing.T) {
	// Validate is a structural question: a declaration is well formed even
	// on a machine where the interpreter is missing, and Probe is what
	// says whether it can run.
	for _, rt := range []string{"bash", "js", "ts", "py"} {
		spec := echoSpec()
		spec.Runtime = rt
		spec.Command = "/nonexistent/script"
		if err := spec.Validate(); err != nil {
			t.Errorf("%s should validate: %v", rt, err)
		}
	}
}

func TestAScriptWithARuntimeProbesAndRuns(t *testing.T) {
	// The two things a declaration with a runtime buys: `adapter check`
	// passes for a path that is not on PATH, and Call spawns it through
	// the interpreter.
	if _, _, err := adapter.RuntimeInterpreter("bash"); err != nil {
		t.Skip("no bash on this machine: ", err)
	}
	dir := t.TempDir()
	path := filepath.Join(dir, "run.sh")
	if err := os.WriteFile(path, []byte("#!/bin/sh\necho native\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	spec := adapter.Spec{
		Name: "s", Command: path, Runtime: "bash",
		Tools: []adapter.ToolSpec{{Name: "go"}},
	}
	if err := spec.Probe(); err != nil {
		t.Fatalf("a script file with a runtime should probe: %v", err)
	}
	res, err := spec.Call(context.Background(), "go", nil)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(res.Stdout, "native") {
		t.Errorf("got %q", res.Stdout)
	}
	if !strings.Contains(res.Command, "bash") || !strings.Contains(res.Command, path) {
		t.Errorf("the resolved command should be reported: %q", res.Command)
	}
}

func TestARuntimeWithNoScriptIsRefusedClearly(t *testing.T) {
	spec := adapter.Spec{Name: "s", Command: "/nonexistent/x.sh", Runtime: "bash",
		Tools: []adapter.ToolSpec{{Name: "go"}}}
	err := spec.Probe()
	if err == nil {
		t.Fatal("a missing script should not probe")
	}
	if !strings.Contains(err.Error(), "not a file") {
		t.Errorf("the message should say what is wrong: %v", err)
	}
}

func TestAScriptOnPathFallsBackWhenAbsentFromDir(t *testing.T) {
	// A runtime adapter may name a bare script that lives on PATH while
	// its working dir is elsewhere: the dir-relative file is tried first,
	// then the documented PATH fallback, not a stat error on the join.
	if _, _, err := adapter.RuntimeInterpreter("bash"); err != nil {
		t.Skip("no bash on this machine: ", err)
	}
	binDir := t.TempDir()
	script := filepath.Join(binDir, "onpath.sh")
	if err := os.WriteFile(script, []byte("#!/bin/sh\necho frompath\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", binDir+string(os.PathListSeparator)+os.Getenv("PATH"))
	spec := adapter.Spec{
		Name: "s", Command: "onpath.sh", Runtime: "bash", Dir: t.TempDir(),
		Tools: []adapter.ToolSpec{{Name: "go"}},
	}
	if err := spec.Probe(); err != nil {
		t.Fatalf("a PATH script with a working dir elsewhere should probe: %v", err)
	}
	res, err := spec.Call(context.Background(), "go", nil)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(res.Stdout, "frompath") {
		t.Errorf("got %q", res.Stdout)
	}
}

func TestARelativeExecutableResolvesAgainstDirWithoutARuntime(t *testing.T) {
	// Regression (Codex on #2504): a declaration with command "./tool",
	// an explicit dir and no runtime used to resolve the executable
	// against the caller's working directory, so a tool that exists only
	// under dir failed before it ever ran. Before commands were resolved
	// in command(), exec resolved the path against cmd.Dir.
	dir := t.TempDir()
	path := filepath.Join(dir, "tool")
	if err := os.WriteFile(path, []byte("#!/bin/sh\necho dirlocal\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	spec := adapter.Spec{
		Name: "d", Command: "./tool", Dir: dir,
		Tools: []adapter.ToolSpec{{Name: "go"}},
	}
	if err := spec.Probe(); err != nil {
		t.Fatalf("an executable under Dir should probe: %v", err)
	}
	res, err := spec.Call(context.Background(), "go", nil)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(res.Stdout, "dirlocal") {
		t.Errorf("got %q", res.Stdout)
	}
}
