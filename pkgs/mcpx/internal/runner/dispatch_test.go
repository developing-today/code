package runner

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"os/exec"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/cache"
	"github.com/developing-today/code/pkgs/mcpx/internal/codegen"
	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
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

func TestParseShebangSplitStringQuoting(t *testing.T) {
	// The Codex P2 on PR #2611: a -S payload is one string env(1)
	// splits with quoting rules, not pre-split words.
	for _, tc := range []struct {
		line string
		want string
	}{
		{`#!/usr/bin/env -S sh -c "printf hello"`, "sh -c printf hello"},
		{`#!/usr/bin/env -S python3 -c 'print("hi")'`, `python3 -c print("hi")`},
		{`#!/usr/bin/env -S sh -c a\ b`, "sh -c a b"},
		{`#!/usr/bin/env --split-string=sh -c "x y"`, "sh -c x y"},
		{`#!/usr/bin/env -Ssh -c "x y"`, "sh -c x y"},
	} {
		got, ok := ParseShebang(tc.line)
		if !ok || strings.Join(got, " ") != tc.want {
			t.Errorf("ParseShebang(%q) = %v, %v; want [%s], true", tc.line, got, ok, tc.want)
		}
	}
	// An unterminated quote is a dispatch error, not a silent
	// mis-split: GNU env errors on it too.
	if _, _, err := decideDispatch(Options{Source: "#!/usr/bin/env -S sh -c \"oops\necho hi\n"}); err == nil {
		t.Error("decideDispatch(unterminated -S quote): want an error")
	}
}

func TestParseShebangChdir(t *testing.T) {
	// The CodeRabbit finding on PR #2611: -C/--chdir was consumed and
	// dropped, so the script ran somewhere its own first line did not
	// name. The operand is carried on the dispatch now.
	for _, tc := range []struct {
		line string
		want string
	}{
		{"#!/usr/bin/env -C /tmp sh", "/tmp"},
		{"#!/usr/bin/env -Ctmp sh", "tmp"},
		{"#!/usr/bin/env --chdir=/tmp sh", "/tmp"},
		{"#!/usr/bin/env --chdir /tmp sh", "/tmp"},
	} {
		sp, ok := parseShebang(tc.line)
		if !ok || strings.Join(sp.tokens, " ") != "sh" || sp.chdir != tc.want {
			t.Errorf("parseShebang(%q) = %v chdir=%q; want [sh] chdir=%q", tc.line, sp.tokens, sp.chdir, tc.want)
		}
	}
}

func TestParseShebangEnvAssignmentWithSlash(t *testing.T) {
	// The CodeRabbit finding on PR #2611: a value containing "/" is
	// still an assignment (PATH=/x:/y), not the command.
	tokens, env, ok := ParseShebangEnv("#!/usr/bin/env PATH=/x:/y sh")
	if !ok || strings.Join(tokens, " ") != "sh" || strings.Join(env, " ") != "PATH=/x:/y" {
		t.Errorf("ParseShebangEnv = %v, %v, %v; want [sh], [PATH=/x:/y], true", tokens, env, ok)
	}
}

func TestRunDispatchedDropsOwnedEnvNames(t *testing.T) {
	// The CodeRabbit finding on PR #2611: a dispatched run must not
	// inherit the run's own contract variables from an outer run --
	// with duplicate keys the first wins, so a stale MCPX_CLIENT path
	// or MCPX_RUNTIME would silently beat the run's own.
	t.Setenv("MCPX_CLIENT", "stale-from-parent")
	t.Setenv("MCPX_RUNTIME", "stale-from-parent")
	dir := t.TempDir()
	f := writeScript(t, dir, "env.sh",
		"#!/bin/sh\necho \"client=[${MCPX_CLIENT-unset}] runtime=$MCPX_RUNTIME\"\n")
	var stdout, stderr bytes.Buffer
	res, err := Run(context.Background(), Options{File: f, Stdout: &stdout, Stderr: &stderr})
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if got := strings.TrimSpace(stdout.String()); got != "client=[unset] runtime=sh" {
		t.Errorf("stdout = %q, want client=[unset] runtime=sh", got)
	}
	if res.ExitCode != 0 {
		t.Errorf("ExitCode = %d, want 0", res.ExitCode)
	}
}

func TestRunShebangChdir(t *testing.T) {
	// End to end for the -C fix: the script runs where its shebang
	// says, ahead of the script.cwd policy.
	dir := t.TempDir()
	target := filepath.Join(dir, "elsewhere")
	if err := os.Mkdir(target, 0o755); err != nil {
		t.Fatal(err)
	}
	f := writeScript(t, dir, "where.sh",
		"#!/usr/bin/env -C "+target+" sh\ntouch ran-in-target-dir\n")
	var stdout, stderr bytes.Buffer
	if _, err := Run(context.Background(), Options{File: f, Stdout: &stdout, Stderr: &stderr}); err != nil {
		t.Fatalf("Run: %v", err)
	}
	// A relative write lands where the shebang says, not next to the
	// script: /bin/pwd is not portable enough to assert on here.
	if _, err := os.Stat(filepath.Join(target, "ran-in-target-dir")); err != nil {
		t.Errorf("marker missing in the -C directory: %v (stderr %q)", err, stderr.String())
	}
	if _, err := os.Stat(filepath.Join(dir, "ran-in-target-dir")); !os.IsNotExist(err) {
		t.Errorf("marker leaked into the script directory")
	}
}

func TestRunDispatchedNoticeNamesSkippedPhases(t *testing.T) {
	// The CodeRabbit finding on runner.go: configured launcher phases
	// are skipped on a dispatched run; the notice must say so instead
	// of staying silent about it.
	dir := t.TempDir()
	f := writeScript(t, dir, "ph.sh", "#!/bin/sh\necho ok\n")
	var stdout, stderr bytes.Buffer
	res, err := Run(context.Background(), Options{
		File: f, Stdout: &stdout, Stderr: &stderr,
		Phases: Phases{Before: []string{"// configured"}},
	})
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if !strings.Contains(res.Notice, "phases") {
		t.Errorf("Notice = %q, want it to name the skipped launcher phases", res.Notice)
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
	d, js, err := decideDispatch(Options{Source: "#!/usr/bin/env node\nexport default () => 1\n"})
	if err != nil || d != nil || js.runtime != "node" {
		t.Errorf("decideDispatch(node shebang) = %v, %q, %v; want nil, \"node\", nil", d, js.runtime, err)
	}
	// A JavaScript shebang with extra arguments keeps raw dispatch:
	// the arguments are part of the named command.
	d, js, err = decideDispatch(Options{Source: "#!/usr/bin/env -S node --inspect main.ts\n"})
	if err != nil || d == nil || js.runtime != "" {
		t.Errorf("decideDispatch(node --inspect) = %v, %q, %v; want a dispatch, \"\", nil", d, js.runtime, err)
	}
	// A non-JavaScript shebang dispatches as before.
	d, js, err = decideDispatch(Options{Source: "#!/bin/sh\necho hi\n"})
	if err != nil || d == nil || js.runtime != "" {
		t.Errorf("decideDispatch(sh) = %v, %q, %v; want a dispatch, \"\", nil", d, js.runtime, err)
	}
	// A JavaScript shebang's env(1) environment rides with the runtime
	// choice instead of being dropped on the harness path.
	d, js, err = decideDispatch(Options{Source: "#!/usr/bin/env -S MODE=prod -u SECRET node\nexport default () => 1\n"})
	if err != nil || d != nil || js.runtime != "node" ||
		strings.Join(js.set, " ") != "MODE=prod" || strings.Join(js.unset, " ") != "SECRET" {
		t.Errorf("decideDispatch(node env) = %v, %+v, %v; want nil, {node [MODE=prod] [SECRET]}, nil", d, js, err)
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
	// Inside double quotes a backslash is literal unless it precedes one
	// of the characters the shell still escapes there: awk needs "\t" to
	// arrive as a backslash and a t, not as a tab or as a bare t (#2611
	// review).
	got, err = SplitCommandLine(`awk -v FS="\t" -f`)
	if err != nil || len(got) != 4 || got[2] != `FS=\t` {
		t.Errorf("double-quoted backslash: %v, %v; want FS=\\t", got, err)
	}
	got, err = SplitCommandLine(`sh -c "printf \"<%s>\" x"`)
	if err != nil || len(got) != 3 || got[2] != `printf "<%s>" x` {
		t.Errorf("escaped quote in double quotes: %v, %v", got, err)
	}
	// Outside quotes a backslash escapes the next character as before.
	got, err = SplitCommandLine(`awk -v FS=\t -f`)
	if err != nil || got[2] != "FS=t" {
		t.Errorf("unquoted escape: %v, %v", got, err)
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

// A JavaScript shebang's -C directory rides into the harness choice:
// dropping it would run the script in the policy directory while the
// raw path honours it (#2611 review).
func TestDecideDispatchJSCarriesChdir(t *testing.T) {
	d, js, err := decideDispatch(Options{Source: "#!/usr/bin/env -S -C /tmp node\nexport default () => 1\n"})
	if err != nil {
		t.Fatalf("decideDispatch: %v", err)
	}
	if d != nil {
		t.Fatalf("decideDispatch(JS shebang with -C) dispatch = %v; want nil (harness path)", d)
	}
	if js.runtime != "node" || js.chdir != "/tmp" {
		t.Errorf("jsChoice = %+v; want runtime node with chdir /tmp carried", js)
	}
}

// The interpreter a shebang names is resolved against the PATH the
// shebang itself assigns, not the parent process's: an executable that
// exists only in the assigned PATH must be found (#2611 review).
func TestRunResolvesAgainstShebangPath(t *testing.T) {
	dir := t.TempDir()
	bin := filepath.Join(dir, "myinterp-probe")
	if err := os.WriteFile(bin, []byte("#!/bin/sh\nexit 0\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	f := writeScript(t, dir, "uses-probe.sh",
		"#!/usr/bin/env -S PATH="+dir+" myinterp-probe\necho hi\n")
	var stdout, stderr bytes.Buffer
	res, err := Run(context.Background(), Options{
		File: f, Stdout: &stdout, Stderr: &stderr,
	})
	if err != nil {
		t.Fatalf("Run(shebang PATH assignment): %v", err)
	}
	if res.Runtime != "myinterp-probe" {
		t.Errorf("Runtime = %q, want the shebang's interpreter", res.Runtime)
	}
	// And without the assignment the same name is not found on the
	// process PATH (it exists nowhere else), proving the assignment --
	// not luck -- did the resolving.
	g := writeScript(t, dir, "no-probe.sh",
		"#!/usr/bin/env myinterp-probe\necho hi\n")
	if _, err := Run(context.Background(), Options{
		File: g, Stdout: &stdout, Stderr: &stderr,
	}); err == nil || !strings.Contains(err.Error(), "not found on PATH") {
		t.Errorf("Run without the PATH assignment: want a not-found error, got %v", err)
	}
}

// Relative PATH entries in a shebang resolve against the directory the
// run moves to (policy + the shebang's -C), not mcpx's own working
// directory: env(1) chdirs first and execvp resolves afterwards, so an
// interpreter that exists only there must be found (#2611 review).
func TestRunShebangRelativePathEntryResolvesPostChdir(t *testing.T) {
	base := t.TempDir()
	proj := filepath.Join(base, "project")
	if err := os.MkdirAll(filepath.Join(proj, "bin"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(proj, "bin", "custom-interp-3201"),
		[]byte("#!/bin/sh\necho post-chdir-resolved\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	f := writeScript(t, base, "go.sh",
		"#!/usr/bin/env -S -C "+proj+" PATH=bin custom-interp-3201\necho unreachable\n")
	var stdout, stderr bytes.Buffer
	res, err := Run(context.Background(), Options{
		File: f, Stdout: &stdout, Stderr: &stderr,
	})
	if err != nil {
		t.Fatalf("Run(relative PATH entry + -C): %v", err)
	}
	if res.Runtime != "custom-interp-3201" || !strings.Contains(stdout.String(), "post-chdir-resolved") {
		t.Errorf("res=%+v stdout=%q; want the interpreter from the -C directory", res, stdout.String())
	}
}

// Assignments after env's "--" terminator still apply: the terminator
// ends options, not the [NAME=VALUE]... part of env's grammar, so
// "-- MODE=prod sh" runs sh with MODE=prod set rather than trying to
// execute a program named MODE=prod (#2611 review).
func TestParseShebangAssignmentsAfterTerminator(t *testing.T) {
	for _, line := range []string{
		"#!/usr/bin/env -- MODE=prod sh",
		"#!/usr/bin/env -S -- MODE=prod sh",
	} {
		sp, ok := parseShebang(line)
		if !ok || strings.Join(sp.tokens, " ") != "sh" || strings.Join(sp.set, " ") != "MODE=prod" {
			t.Errorf("parseShebang(%q) = %v set=%v, %v; want [sh] set=[MODE=prod], true", line, sp.tokens, sp.set, ok)
		}
	}
}

// A -S payload reaches the splitter untouched: whitespace inside
// quotes is part of the argument, so 'a  b' must not collapse to
// 'a b' on the way through fields-splitting (#2611 review).
func TestParseShebangSplitStringPreservesQuotedWhitespace(t *testing.T) {
	line := `#!/usr/bin/env -S sh -c 'printf "<%s>" "a  b"'`
	want := []string{"sh", "-c", `printf "<%s>" "a  b"`}
	got, ok := ParseShebang(line)
	if !ok || strings.Join(got, "\x00") != strings.Join(want, "\x00") {
		t.Errorf("ParseShebang(%q) = %q, %v; want %q, true", line, got, ok, want)
	}
}

// An explicitly empty PATH (PATH=) is not an absent PATH: it stays
// empty (search the effective directory only), while -i and -u PATH
// make PATH absent and earn execvp's /bin:/usr/bin default
// (#2611 review).
func TestShebangPathEmptyVersusAbsent(t *testing.T) {
	t.Setenv("PATH", "/ inherited")
	if got := shebangPath(&shebangEnv{}); got != "/ inherited" {
		t.Errorf("shebangPath(inherit) = %q, want the inherited PATH", got)
	}
	if got := shebangPath(&shebangEnv{set: []string{"PATH="}}); got != "" {
		t.Errorf("shebangPath(PATH=) = %q, want empty (effective directory only)", got)
	}
	if got := shebangPath(&shebangEnv{clear: true}); got != "/bin:/usr/bin" {
		t.Errorf("shebangPath(-i) = %q, want the execvp default", got)
	}
	if got := shebangPath(&shebangEnv{unset: []string{"PATH"}}); got != "/bin:/usr/bin" {
		t.Errorf("shebangPath(-u PATH) = %q, want the execvp default", got)
	}
	if got := shebangPath(&shebangEnv{clear: true, set: []string{"PATH="}}); got != "" {
		t.Errorf("shebangPath(-i PATH=) = %q, want empty: the assignment re-establishes PATH, empty", got)
	}
}

// End to end on the empty-PATH rule: an interpreter that exists only
// in the run's effective directory is found through PATH=, where the
// /bin:/usr/bin default would have missed it (#2611 review).
func TestRunShebangExplicitEmptyPathUsesEffectiveDir(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "only-here-interp"),
		[]byte("#!/bin/sh\necho empty-path-resolved\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	f := writeScript(t, dir, "empty-path.sh",
		"#!/usr/bin/env -S PATH= only-here-interp\necho unreachable\n")
	var stdout, stderr bytes.Buffer
	res, err := Run(context.Background(), Options{
		File: f, CwdPolicy: "script", Stdout: &stdout, Stderr: &stderr,
	})
	if err != nil {
		t.Fatalf("Run(PATH= shebang): %v", err)
	}
	if res.Runtime != "only-here-interp" || !strings.Contains(stdout.String(), "empty-path-resolved") {
		t.Errorf("res=%+v stdout=%q; want the interpreter from the effective directory", res, stdout.String())
	}
}

// TestSplitEnvStringUsesEnvSemantics is the Codex finding on
// PR #2611 (dispatch.go): the -S payload was split with the
// configured-interpreter (shell) tokenizer, whose escape rules
// turned the coreutils manual's own example FS=\"\\t\" into a
// literal FS="t" and left ${VAR} unexpanded.
func TestSplitEnvStringUsesEnvSemantics(t *testing.T) {
	cases := []struct {
		payload string
		want    []string
	}{
		{`awk -v FS=\"\t\" -f`, []string{"awk", "-v", `FS="` + "\t" + `"`, "-f"}},
		{`sh -c "printf hello"`, []string{"sh", "-c", "printf hello"}},
		// \_ outside quotes is an argument separator in env(1),
		// not a space: `python3\_-u` must dispatch python3 with -u
		// (Codex review, PR #2611; verified against GNU env).
		{`echo a\_b`, []string{"echo", "a", "b"}},
		{`python3\_-u script.py`, []string{"python3", "-u", "script.py"}},
		// Inside double quotes \_ is a literal space.
		{`echo "a\_b"`, []string{"echo", "a b"}},
		// An unquoted # that begins an argument starts a comment;
		// a # inside an argument is literal (Codex review, PR #2611).
		{`sh # interpreter comment`, []string{"sh"}},
		{`echo a#b`, []string{"echo", "a#b"}},
		{"echo 'single quoted'", []string{"echo", "single quoted"}},
		// Inside single quotes only \' and \\ are escapes; every other
		// backslash is literal, or a regexp like r"\d" loses its
		// backslash and matches something else (#2611 review).
		{`python3 -c 'import re; print(bool(re.search(r"\d", "5")))'`,
			[]string{"python3", "-c", `import re; print(bool(re.search(r"\d", "5")))`}},
		{`echo 'it\'s'`, []string{"echo", "it's"}},
	}
	for _, c := range cases {
		got, err := SplitEnvString(c.payload)
		if err != nil {
			t.Errorf("SplitEnvString(%q): %v", c.payload, err)
			continue
		}
		if len(got) != len(c.want) {
			t.Errorf("SplitEnvString(%q) = %q, want %q", c.payload, got, c.want)
			continue
		}
		for i := range got {
			if got[i] != c.want[i] {
				t.Errorf("SplitEnvString(%q)[%d] = %q, want %q (all: %q)", c.payload, i, got[i], c.want[i], got)
			}
		}
	}
	if _, err := SplitEnvString(`echo ${UNTERMINATED`); err == nil {
		t.Error("SplitEnvString(unterminated ${): want an error")
	}
}

// TestSplitEnvStringExpandsFromTheInheritedEnvironment is the Codex
// finding on PR #2611: env expands ${VAR} BEFORE applying the
// payload's own assignments, so a preceding MODE=prod in the payload
// must not shadow the inherited MODE. Verified against GNU env:
// MODE=old env -S 'MODE=prod printf ${MODE}' prints "old".
func TestSplitEnvStringExpandsFromTheInheritedEnvironment(t *testing.T) {
	t.Setenv("MODE", "old")
	got, err := SplitEnvString(`MODE=prod printf ${MODE}`)
	if err != nil {
		t.Fatal(err)
	}
	want := []string{"MODE=prod", "printf", "old"}
	if len(got) != len(want) {
		t.Fatalf("SplitEnvString = %q, want %q", got, want)
	}
	for i := range got {
		if got[i] != want[i] {
			t.Fatalf("SplitEnvString = %q, want %q", got, want)
		}
	}
}

// TestSplitEnvStringEmptyUnquotedExpansionStartsNoArgument is the
// Codex finding on PR #2611 (re-review): an unset variable's unquoted
// expansion must not create an empty argument — env(1) omits the word
// — while an explicitly quoted empty argument survives.
func TestSplitEnvStringEmptyUnquotedExpansionStartsNoArgument(t *testing.T) {
	// Deliberately never set in this process.
	const unset = "MCPX_SPLIT_TEST_UNSET_3201"
	cases := []struct {
		payload string
		want    []string
	}{
		{`python3 ${` + unset + `}`, []string{"python3"}},
		{`python3 ${` + unset + `} script.py`, []string{"python3", "script.py"}},
		{`python3 "${` + unset + `}"`, []string{"python3", ""}},
		{`echo a${` + unset + `}b`, []string{"echo", "ab"}},
		{`${` + unset + `}`, nil},
	}
	for _, c := range cases {
		got, err := SplitEnvString(c.payload)
		if err != nil {
			t.Errorf("SplitEnvString(%q): %v", c.payload, err)
			continue
		}
		if len(got) != len(c.want) {
			t.Errorf("SplitEnvString(%q) = %q, want %q", c.payload, got, c.want)
			continue
		}
		for i := range got {
			if got[i] != c.want[i] {
				t.Errorf("SplitEnvString(%q)[%d] = %q, want %q (all: %q)", c.payload, i, got[i], c.want[i], got)
			}
		}
	}
}

// TestDispatchedWorkdirRunLeasesItsDirectory is the Codex finding on
// PR #2611 (re-review): a dispatched run's allocated runner directory
// stayed unleased for the whole execution, so another run's cache
// pruning could delete it mid-run. The directory must be leased while
// the run is live and released when it finishes.
func TestDispatchedWorkdirRunLeasesItsDirectory(t *testing.T) {
	dir := t.TempDir()
	root := t.TempDir()
	marker := filepath.Join(dir, "started")
	script := writeScript(t, dir, "job.sh",
		fmt.Sprintf("#!/bin/sh\npwd > %s\nsleep 2\n", marker))
	done := make(chan error, 1)
	go func() {
		var stdout, stderr bytes.Buffer
		_, err := Run(context.Background(), Options{
			File: script, CwdPolicy: "workdir", WorkRoot: root,
			Stdout: &stdout, Stderr: &stderr,
		})
		done <- err
	}()
	deadline := time.Now().Add(10 * time.Second)
	var cwd string
	for time.Now().Before(deadline) {
		if b, err := os.ReadFile(marker); err == nil && len(b) > 0 {
			cwd = strings.TrimSpace(string(b))
			break
		}
		time.Sleep(20 * time.Millisecond)
	}
	if cwd == "" {
		t.Fatal("the dispatched run never reported its directory")
	}
	if !cache.Leased(cwd, defaults.CacheLeaseStaleAge) {
		t.Fatalf("dispatched workdir %q is not leased while the run is live", cwd)
	}
	if err := <-done; err != nil {
		t.Fatalf("Run(workdir dispatch): %v", err)
	}
	if cache.Leased(cwd, defaults.CacheLeaseStaleAge) {
		t.Fatalf("dispatched workdir %q is still leased after the run finished", cwd)
	}
}

// TestDispatchedWorkdirPolicyAllocatesARunnerDirectory is the Codex
// finding on PR #2611: a file-backed dispatched run under
// script.cwd=workdir with only a WorkRoot supplied ran in the
// script's own directory, making workdir behave as script. The run
// must land in a runner directory under the root.
func TestDispatchedWorkdirPolicyAllocatesARunnerDirectory(t *testing.T) {
	dir := t.TempDir()
	root := t.TempDir()
	script := writeScript(t, dir, "job.sh", "#!/bin/sh\npwd\n")
	var stdout, stderr bytes.Buffer
	_, err := Run(context.Background(), Options{
		File: script, CwdPolicy: "workdir", WorkRoot: root,
		Stdout: &stdout, Stderr: &stderr,
	})
	if err != nil {
		t.Fatalf("Run(workdir dispatch): %v (stderr %q)", err, stderr.String())
	}
	got := strings.TrimSpace(stdout.String())
	if got == "" || !strings.HasPrefix(got, root) {
		t.Fatalf("dispatched workdir run executed in %q, want a directory under WorkRoot %q", got, root)
	}
	if got == dir {
		t.Fatalf("dispatched workdir run executed in the script directory %q", dir)
	}
}

// A JavaScript shebang's runtime may exist only on the PATH the shebang
// sets, and that PATH is applied after the run's directory is known.
// Resolve runs first, against mcpx's inherited PATH, and must not reject
// the runtime before the shebang's own PATH is consulted (#2611 review,
// Major).
func TestRunJSRuntimeShebangFoundOnlyOnShebangPath(t *testing.T) {
	dir := t.TempDir()
	// A stand-in runtime of the node kind: it has to be found and
	// executed, not be a real Node, because the point is resolution.
	fake := writeScript(t, dir, "node", "#!/bin/sh\necho fake-node-ran\nexit 0\n")
	_ = fake
	script := writeScript(t, dir, "app.mjs",
		"#!/usr/bin/env -S PATH="+dir+" node\nexport default () => 1\n")
	var stdout, stderr bytes.Buffer
	res, err := Run(context.Background(), Options{
		File: script, Stdout: &stdout, Stderr: &stderr,
		ClientSource: codegen.Module(nil, "", ""),
	})
	if err != nil {
		t.Fatalf("Run(node on the shebang's PATH only): %v", err)
	}
	if res.Runtime != "node" {
		t.Errorf("Runtime = %q, want node", res.Runtime)
	}
	if !strings.Contains(stdout.String(), "fake-node-ran") {
		t.Errorf("stdout = %q; the shebang's own node must have run", stdout.String())
	}
}

// A configured script.interpreter is looked up against the directory the
// run moves to, the way a shebang's is: a relative PATH entry means the
// run's directory, so an interpreter that exists only there must be
// found (#2611 review).
func TestRunInterpreterResolvedInRunDirectory(t *testing.T) {
	base := t.TempDir()
	scriptDir := filepath.Join(base, "project")
	if err := os.MkdirAll(filepath.Join(scriptDir, "bin"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(scriptDir, "bin", "custom-interp-3201"),
		[]byte("#!/bin/sh\necho interpreter-from-run-dir\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	f := writeScript(t, scriptDir, "run.sh", "echo unreachable\n")
	t.Setenv("PATH", "bin")
	var stdout, stderr bytes.Buffer
	res, err := Run(context.Background(), Options{
		File: f, Dir: scriptDir, CwdPolicy: "script", Interpreter: "custom-interp-3201",
		Stdout: &stdout, Stderr: &stderr,
	})
	if err != nil {
		t.Fatalf("Run(relative PATH entry, script.interpreter): %v", err)
	}
	if res.ExitCode != 0 || !strings.Contains(stdout.String(), "interpreter-from-run-dir") {
		t.Errorf("res=%+v stdout=%q; want the interpreter from the run directory", res, stdout.String())
	}
}

// An env option mcpx does not implement fails the run instead of being
// dropped: dropping it runs a different command than the shebang names,
// and -0/-v are not inert (#2611 review).
func TestRunRejectsUnsupportedEnvOption(t *testing.T) {
	dir := t.TempDir()
	for _, opt := range []string{"--bogus", "-0", "-v", "--debug"} {
		f := writeScript(t, dir, "opt.sh", "#!/usr/bin/env -S "+opt+" sh\necho unreachable\n")
		var stdout, stderr bytes.Buffer
		_, err := Run(context.Background(), Options{
			File: f, Stdout: &stdout, Stderr: &stderr,
		})
		if err == nil || !strings.Contains(err.Error(), "not a supported env option") {
			t.Errorf("option %s: want a precise option error, got %v", opt, err)
		}
		if strings.Contains(stdout.String(), "unreachable") {
			t.Errorf("option %s: the script ran despite the unsupported option", opt)
		}
	}
}
