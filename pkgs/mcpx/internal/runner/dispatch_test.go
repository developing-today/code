package runner

import (
	"bytes"
	"context"
	"os"
	"os/exec"

	"github.com/dezren39/mcpx/internal/codegen"
	"path/filepath"
	"strings"
	"testing"
)

func TestParseShebang(t *testing.T) {
	for _, tc := range []struct {
		line string
		want []string
		ok   bool
	}{
		{"#!/bin/sh", []string{"/bin/sh"}, true},
		{"#!/usr/bin/env python3", []string{"python3"}, true},
		{"#!/usr/bin/env -S python3 -u", []string{"python3", "-u"}, true},
		{"#!/usr/bin/env FOO=1 python3", []string{"python3"}, true},
		{"#!/bin/sh -e", []string{"/bin/sh", "-e"}, true},
		{"#!", nil, true},
		{"# just a comment", nil, false},
		{"echo hi", nil, false},
		{"", nil, false},
	} {
		got, ok := ParseShebang(tc.line)
		if ok != tc.ok || strings.Join(got, " ") != strings.Join(tc.want, " ") {
			t.Errorf("ParseShebang(%q) = %v, %v; want %v, %v", tc.line, got, ok, tc.want, tc.ok)
		}
	}
}

func TestParseShebangEnvAssignments(t *testing.T) {
	tokens, env, ok := ParseShebangEnv("#!/usr/bin/env -S MODE=prod sh")
	if !ok || strings.Join(tokens, " ") != "sh" || strings.Join(env, " ") != "MODE=prod" {
		t.Errorf("ParseShebangEnv = %v, %v, %v; want [sh], [MODE=prod], true", tokens, env, ok)
	}
	tokens, env, ok = ParseShebangEnv("#!/usr/bin/env FOO=1 BAR=two python3 -u")
	if !ok || strings.Join(tokens, " ") != "python3 -u" || strings.Join(env, " ") != "FOO=1 BAR=two" {
		t.Errorf("ParseShebangEnv = %v, %v, %v; want [python3 -u], [FOO=1 BAR=two], true", tokens, env, ok)
	}
	// A non-env shebang has no assignments to preserve.
	_, env, _ = ParseShebangEnv("#!/bin/sh")
	if len(env) != 0 {
		t.Errorf("env = %v, want none", env)
	}
}

func TestParseShebangEnvOptions(t *testing.T) {
	for _, tc := range []struct {
		line  string
		want  string
		set   string
		unset string
		clear bool
	}{
		{"#!/usr/bin/env -S -u SECRET sh", "sh", "", "SECRET", false},
		{"#!/usr/bin/env --unset=HOME sh", "sh", "", "HOME", false},
		{"#!/usr/bin/env -uSECRET sh", "sh", "", "SECRET", false},
		{"#!/usr/bin/env -C /tmp sh", "sh", "", "", false},
		{"#!/usr/bin/env --chdir=/tmp sh", "sh", "", "", false},
		{"#!/usr/bin/env -i sh", "sh", "", "", true},
		{"#!/usr/bin/env --ignore-environment sh", "sh", "", "", true},
		{"#!/usr/bin/env -S MODE=prod -u SECRET sh", "sh", "MODE=prod", "SECRET", false},
		{"#!/usr/bin/env -- sh", "sh", "", "", false},
	} {
		sp, ok := parseShebang(tc.line)
		if !ok ||
			strings.Join(sp.tokens, " ") != tc.want ||
			strings.Join(sp.set, " ") != tc.set ||
			strings.Join(sp.unset, " ") != tc.unset ||
			sp.clear != tc.clear {
			t.Errorf("parseShebang(%q) = %v set=%v unset=%v clear=%v; want tokens=%q set=%q unset=%q clear=%v",
				tc.line, sp.tokens, sp.set, sp.unset, sp.clear, tc.want, tc.set, tc.unset, tc.clear)
		}
	}
}

func TestFilterEnv(t *testing.T) {
	got := filterEnv([]string{"A=1", "B=2", "A=3"}, []string{"A"})
	if strings.Join(got, " ") != "B=2" {
		t.Errorf("filterEnv = %v, want [B=2]", got)
	}
	// With nothing to drop the caller's slice is passed through as is.
	in := []string{"A=1"}
	if got := filterEnv(in, nil); &got[0] != &in[0] {
		t.Errorf("filterEnv with no unset names copied the environment")
	}
}

func TestRunEnvShebangUnsetAndIgnore(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("MCPX_TEST_DROP", "inherited")

	dropped := writeScript(t, dir, "dropped.sh",
		"#!/usr/bin/env -S -u MCPX_TEST_DROP sh\necho \"drop=${MCPX_TEST_DROP-unset}\"\n")
	var stdout, stderr bytes.Buffer
	if _, err := Run(context.Background(), Options{File: dropped, Stdout: &stdout, Stderr: &stderr}); err != nil {
		t.Fatalf("Run: %v", err)
	}
	if got := strings.TrimSpace(stdout.String()); got != "drop=unset" {
		t.Errorf("stdout = %q, want drop=unset", got)
	}

	ignored := writeScript(t, dir, "ignored.sh",
		"#!/usr/bin/env -i sh\necho \"drop=${MCPX_TEST_DROP-unset} runtime=$MCPX_RUNTIME\"\n")
	stdout.Reset()
	if _, err := Run(context.Background(), Options{File: ignored, Stdout: &stdout, Stderr: &stderr}); err != nil {
		t.Fatalf("Run: %v", err)
	}
	if got := strings.TrimSpace(stdout.String()); got != "drop=unset runtime=sh" {
		t.Errorf("stdout = %q, want drop=unset runtime=sh", got)
	}
}

func TestRunEnvShebangAssignmentsReachTheScript(t *testing.T) {
	dir := t.TempDir()
	f := writeScript(t, dir, "mode.sh",
		"#!/usr/bin/env -S MODE=prod sh\necho \"mode=$MODE\"\n")
	var stdout, stderr bytes.Buffer
	res, err := Run(context.Background(), Options{
		File: f, Stdout: &stdout, Stderr: &stderr,
	})
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if res.ExitCode != 0 || !strings.Contains(stdout.String(), "mode=prod") {
		t.Errorf("res=%+v stdout=%q; the shebang's MODE=prod assignment must reach the script", res, stdout.String())
	}
}

func TestDecideDispatchJSRuntimeShebang(t *testing.T) {
	// A plain JavaScript-runtime shebang must NOT divert to raw
	// dispatch: the harness runs it, under the shebang's runtime.
	d, jsRuntime, err := decideDispatch(Options{Source: "#!/usr/bin/env node\nexport default () => 1\n"})
	if err != nil || d != nil || jsRuntime != "node" {
		t.Errorf("decideDispatch(node shebang) = %v, %q, %v; want nil, \"node\", nil", d, jsRuntime, err)
	}
	// A JavaScript shebang with extra arguments keeps raw dispatch:
	// the arguments are part of the named command.
	d, jsRuntime, err = decideDispatch(Options{Source: "#!/usr/bin/env -S node --inspect main.ts\n"})
	if err != nil || d == nil || jsRuntime != "" {
		t.Errorf("decideDispatch(node --inspect) = %v, %q, %v; want a dispatch, \"\", nil", d, jsRuntime, err)
	}
	// A non-JavaScript shebang dispatches as before.
	d, jsRuntime, err = decideDispatch(Options{Source: "#!/bin/sh\necho hi\n"})
	if err != nil || d == nil || jsRuntime != "" {
		t.Errorf("decideDispatch(sh) = %v, %q, %v; want a dispatch, \"\", nil", d, jsRuntime, err)
	}
}

func TestRunJSRuntimeShebangKeepsHarness(t *testing.T) {
	// The Codex finding on PR #2611, end to end: a Node .mjs file whose
	// shebang names node must still go through the generated launcher,
	// so its default export is called and its return value printed --
	// raw dispatch ran the file as a plain script and produced neither.
	if _, err := exec.LookPath("node"); err != nil {
		t.Skip("node is not installed")
	}
	dir := t.TempDir()
	f := writeScript(t, dir, "main.mjs",
		"#!/usr/bin/env node\nexport default () => \"expected-result\"\n")
	var stdout, stderr bytes.Buffer
	res, err := Run(context.Background(), Options{
		File: f, Stdout: &stdout, Stderr: &stderr,
		// The launcher imports the generated client; an empty one does
		// not export what it names, which is a failure of the fixture,
		// not of the run.
		ClientSource: codegen.Module(nil, "", ""),
	})
	if err != nil {
		t.Fatalf("Run: %v (stderr %q)", err, stderr.String())
	}
	if res.ExitCode != 0 || !strings.Contains(stdout.String(), "expected-result") {
		t.Errorf("res=%+v stdout=%q stderr=%q; the harness must call the default export and print its result", res, stdout.String(), stderr.String())
	}
	if res.Runtime != "node" {
		t.Errorf("Runtime = %q, want node (the shebang's choice)", res.Runtime)
	}
}

func TestSplitCommandLine(t *testing.T) {
	got, err := SplitCommandLine(`python3 -u`)
	if err != nil || strings.Join(got, " ") != "python3 -u" {
		t.Errorf("plain: %v, %v", got, err)
	}
	got, err = SplitCommandLine(`"/opt/my dir/python" -x 'a b'`)
	if err != nil || len(got) != 3 || got[0] != "/opt/my dir/python" || got[2] != "a b" {
		t.Errorf("quoted: %v, %v", got, err)
	}
	if _, err := SplitCommandLine(`"unterminated`); err == nil {
		t.Error("unterminated quote: want an error")
	}
}

func TestResolveCwdPolicies(t *testing.T) {
	script := filepath.Join("/proj", "scripts", "a.sh")
	for _, tc := range []struct {
		policy, want string
		wantErr      bool
	}{
		{"", "/caller", false},
		{"caller", "/caller", false},
		{"script", "/proj/scripts", false},
		{"config", "/cfg", false},
		{"workdir", "/work", false},
		{"bogus", "", true},
	} {
		got, err := ResolveCwd(tc.policy, "/caller", script, "/cfg", "/work")
		if tc.wantErr {
			if err == nil {
				t.Errorf("policy %q: want an error", tc.policy)
			}
			continue
		}
		if err != nil || got != tc.want {
			t.Errorf("policy %q = %q, %v; want %q", tc.policy, got, err, tc.want)
		}
	}
	if _, err := ResolveCwd("config", "/caller", script, "", "/work"); err == nil {
		t.Error("config with no config directory: want an explicit error")
	}
}

func writeScript(t *testing.T, dir, name, body string) string {
	t.Helper()
	p := filepath.Join(dir, name)
	if err := os.WriteFile(p, []byte(body), 0o755); err != nil {
		t.Fatal(err)
	}
	return p
}

func TestRunShebangFileRunsUnderItsInterpreter(t *testing.T) {
	dir := t.TempDir()
	f := writeScript(t, dir, "hello.sh",
		"#!/bin/sh\necho \"out-$MCPX_RUNTIME-$1\"\necho \"client=[$MCPX_CLIENT]\"\necho \"ep=$MCPX_ENDPOINT\"\nexit 3\n")
	var stdout, stderr bytes.Buffer
	res, err := Run(context.Background(), Options{
		File: f, Args: []string{"hello"},
		Env:    map[string]string{"MCPX_ENDPOINT": "http://example.invalid:9999"},
		Stdout: &stdout, Stderr: &stderr,
	})
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if res.ExitCode != 3 {
		t.Errorf("ExitCode = %d, want 3 (propagated, not errored)", res.ExitCode)
	}
	if res.Runtime != "sh" || res.Client != "" {
		t.Errorf("Result = %+v, want Runtime sh and no client", res)
	}
	out := stdout.String()
	for _, want := range []string{"out-sh-hello", "client=[]", "ep=http://example.invalid:9999"} {
		if !strings.Contains(out, want) {
			t.Errorf("stdout %q missing %q", out, want)
		}
	}
	if res.Notice == "" || !strings.Contains(stderr.String(), "no generated mcpx client") {
		t.Errorf("the missing client must be stated once: Notice=%q stderr=%q", res.Notice, stderr.String())
	}
}

func TestRunShebangSnippet(t *testing.T) {
	var stdout, stderr bytes.Buffer
	res, err := Run(context.Background(), Options{
		Source: "#!/bin/sh\necho snippet-ok\n",
		Stdout: &stdout, Stderr: &stderr,
	})
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if res.ExitCode != 0 || !strings.Contains(stdout.String(), "snippet-ok") {
		t.Errorf("res=%+v stdout=%q", res, stdout.String())
	}
}

func TestDispatchPrecedenceShebangBeatsInterpreterBeatsRuntime(t *testing.T) {
	dir := t.TempDir()
	// A bogus configured interpreter would fail resolution if it were
	// consulted; the shebang must win without consulting it.
	f := writeScript(t, dir, "wins.sh", "#!/bin/sh\necho shebang-won\n")
	var stdout, stderr bytes.Buffer
	res, err := Run(context.Background(), Options{
		File: f, Interpreter: "mcpx-no-such-interpreter-3201",
		Stdout: &stdout, Stderr: &stderr,
	})
	if err != nil || !strings.Contains(stdout.String(), "shebang-won") {
		t.Fatalf("shebang precedence: res=%+v err=%v stdout=%q", res, err, stdout.String())
	}
	// No shebang: the configured interpreter runs the file.
	g := writeScript(t, dir, "plain.txt", "echo interpreter-won\n")
	stdout.Reset()
	res, err = Run(context.Background(), Options{
		File: g, Interpreter: "sh",
		Stdout: &stdout, Stderr: &stderr,
	})
	if err != nil || !strings.Contains(stdout.String(), "interpreter-won") || res.Runtime != "sh" {
		t.Fatalf("interpreter dispatch: res=%+v err=%v stdout=%q", res, err, stdout.String())
	}
}

func TestRunInterpreterSnippetWithoutShebang(t *testing.T) {
	var stdout, stderr bytes.Buffer
	res, err := Run(context.Background(), Options{
		Source: "echo via-interpreter\n", Interpreter: "sh",
		Stdout: &stdout, Stderr: &stderr,
	})
	if err != nil || res.ExitCode != 0 || !strings.Contains(stdout.String(), "via-interpreter") {
		t.Fatalf("res=%+v err=%v stdout=%q", res, err, stdout.String())
	}
}

func TestRunCwdPolicyScript(t *testing.T) {
	base := t.TempDir()
	scriptDir := filepath.Join(base, "scripts")
	if err := os.MkdirAll(scriptDir, 0o755); err != nil {
		t.Fatal(err)
	}
	f := writeScript(t, scriptDir, "where.sh", "#!/bin/sh\npwd\n")
	run := func(policy string) string {
		var stdout, stderr bytes.Buffer
		_, err := Run(context.Background(), Options{
			File: f, Dir: base, CwdPolicy: policy,
			Stdout: &stdout, Stderr: &stderr,
		})
		if err != nil {
			t.Fatalf("policy %s: %v", policy, err)
		}
		got, _ := filepath.EvalSymlinks(strings.TrimSpace(stdout.String()))
		return got
	}
	wantScript, _ := filepath.EvalSymlinks(scriptDir)
	wantBase, _ := filepath.EvalSymlinks(base)
	if got := run("script"); got != wantScript {
		t.Errorf("script.cwd=script: pwd = %q, want %q", got, wantScript)
	}
	if got := run("caller"); got != wantBase {
		t.Errorf("script.cwd=caller: pwd = %q, want %q", got, wantBase)
	}
}
