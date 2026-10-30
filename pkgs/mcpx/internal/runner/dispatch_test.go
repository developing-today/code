package runner

import (
	"bytes"
	"context"
	"os"
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
