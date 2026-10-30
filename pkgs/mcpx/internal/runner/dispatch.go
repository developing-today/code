// Interpreter dispatch for non-TypeScript scripts (#2167, parts 1-3 of
// #1919).
//
// Dispatch order, first match wins:
//
//  1. The source or file starts with a shebang (#!): that interpreter,
//     verbatim. A shebang is an instruction inside the program itself, so
//     it beats every configuration surface -- including script.runtime:
//     a file that names its own interpreter is not overridden by a
//     machine-wide preference for a different one.
//  2. script.interpreter: a configured command line, with the script
//     path appended.
//  3. script.language / file extension: RESERVED for #1917. Nothing
//     diverts here yet, by design -- the rank exists so the order does
//     not change when it lands.
//  4. The configured TypeScript runtime: today's behaviour, unchanged
//     when everything above is unset.
//
// A dispatched (non-JavaScript) run gets the harness posture, not the
// harness: MCPX_ENDPOINT / MCPX_SESSION / MCPX_SOCKET arrive through
// Options.Env exactly as they do for a TypeScript run, MCPX_RUNTIME
// names the interpreter, MCPX_ENTRY names the script, the caller's args
// follow the script path, and stdout/stderr/exit/timeout handling is the
// same machinery. What it does not get is the generated client -- there
// is no TypeScript module a shell script could import -- and that
// absence is stated once per run (Result.Notice, and a line on stderr),
// not left for the script to discover. script.permissions profiles are
// JavaScript-runtime flags; no shell interpreter can enforce them, so
// they are not applied to a dispatched run and the notice says so.
package runner

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	"github.com/dezren39/mcpx/internal/defaults"
	"github.com/dezren39/mcpx/internal/logging"
)

// CwdPolicies are the script.cwd values, in the order the setting
// documents them.
var CwdPolicies = []string{"caller", "script", "config", "workdir"}

// ResolveCwd applies a script.cwd policy. callerDir is Options.Dir (the
// caller's explicit directory, empty meaning inherit), scriptPath the
// final entry script, configDir the directory holding the mcpx
// configuration in force, and workDir the runner's working directory
// for the run (where a snippet and the generated client live).
func ResolveCwd(policy, callerDir, scriptPath, configDir, workDir string) (string, error) {
	switch policy {
	case "", "caller":
		return callerDir, nil
	case "script":
		if scriptPath == "" {
			return "", errors.New("script.cwd=script but the run has no script file")
		}
		return filepath.Dir(scriptPath), nil
	case "config":
		if configDir == "" {
			return "", errors.New("script.cwd=config but no configuration directory is known for this run")
		}
		return configDir, nil
	case "workdir":
		if workDir == "" {
			return "", errors.New("script.cwd=workdir but the run has no working directory")
		}
		return workDir, nil
	}
	return "", fmt.Errorf("script.cwd %q is not one of %s", policy, strings.Join(CwdPolicies, ", "))
}

// dispatch is a resolved non-JavaScript interpreter invocation: the
// binary, the arguments that precede the script path, and where the
// decision came from.
type dispatch struct {
	bin    string
	args   []string
	name   string // interpreter name reported as the run's runtime
	source string // "shebang" or "interpreter"
}

// ParseShebang splits a #! line into the interpreter command it names.
// The /usr/bin/env form is unwrapped the way the kernel cannot: env's
// own flags (-S, -i) and VAR=value assignments are consumed, and what
// remains is the command. ok is false when the line is not a shebang.
func ParseShebang(line string) (tokens []string, ok bool) {
	line = strings.TrimRight(line, "\r\n")
	if !strings.HasPrefix(line, "#!") {
		return nil, false
	}
	fields := strings.Fields(strings.TrimSpace(line[2:]))
	if len(fields) == 0 {
		return nil, true
	}
	if filepath.Base(fields[0]) == "env" {
		fields = fields[1:]
		for len(fields) > 0 {
			f := fields[0]
			if strings.HasPrefix(f, "-") || (strings.Contains(f, "=") && !strings.Contains(f, "/")) {
				fields = fields[1:]
				continue
			}
			break
		}
	}
	return fields, true
}

// SplitCommandLine splits a configured command line (script.interpreter)
// into words, honouring single and double quotes and backslash escapes.
// It is deliberately small: an interpreter command line is a handful of
// words, and anything fancier belongs in a script of its own.
func SplitCommandLine(s string) ([]string, error) {
	var out []string
	var word strings.Builder
	quoted := byte(0)
	escaped := false
	started := false
	flush := func() {
		if started {
			out = append(out, word.String())
			word.Reset()
			started = false
		}
	}
	for i := 0; i < len(s); i++ {
		c := s[i]
		switch {
		case escaped:
			word.WriteByte(c)
			escaped = false
			started = true
		case c == '\\' && quoted != '\'':
			escaped = true
		case quoted != 0:
			if c == quoted {
				quoted = 0
			} else {
				word.WriteByte(c)
			}
			started = true
		case c == '\'' || c == '"':
			quoted = c
			started = true
		case c == ' ' || c == '\t':
			flush()
		default:
			word.WriteByte(c)
			started = true
		}
	}
	if quoted != 0 {
		return nil, fmt.Errorf("unterminated %q in command line %q", string(quoted), s)
	}
	flush()
	return out, nil
}

func firstLineOf(s string) string {
	if i := strings.IndexByte(s, '\n'); i >= 0 {
		return s[:i]
	}
	return s
}

func fileFirstLine(path string) (string, error) {
	f, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer f.Close()
	line, err := bufio.NewReader(io.LimitReader(f, 8192)).ReadString('\n')
	if err != nil && line == "" {
		return "", err
	}
	return line, nil
}

// decideDispatch applies ranks 1 and 2 of the dispatch order. A nil
// dispatch means the run falls through to the TypeScript runtime.
func decideDispatch(opts Options) (*dispatch, error) {
	first := ""
	if opts.Source != "" {
		first = firstLineOf(opts.Source)
	} else if opts.File != "" {
		// A read error here is not fatal to the decision: the normal
		// path surfaces it with better context when it opens the file
		// for real. Only a shebang that IS readable diverts the run.
		if l, err := fileFirstLine(opts.File); err == nil {
			first = l
		}
	}
	if tokens, ok := ParseShebang(first); ok {
		if len(tokens) == 0 {
			return nil, errors.New("runner: the script's shebang line (#!) names no interpreter")
		}
		return dispatchFromTokens("shebang", tokens)
	}
	if strings.TrimSpace(opts.Interpreter) != "" {
		tokens, err := SplitCommandLine(opts.Interpreter)
		if err != nil {
			return nil, fmt.Errorf("runner: script.interpreter: %w", err)
		}
		if len(tokens) == 0 {
			return nil, errors.New("runner: script.interpreter is set but names no command")
		}
		return dispatchFromTokens("interpreter", tokens)
	}
	return nil, nil
}

func dispatchFromTokens(source string, tokens []string) (*dispatch, error) {
	bin := tokens[0]
	resolved := bin
	if !strings.Contains(bin, "/") {
		p, err := exec.LookPath(bin)
		if err != nil {
			return nil, fmt.Errorf("runner: %s interpreter %q not found on PATH: %w", source, bin, err)
		}
		resolved = p
	}
	return &dispatch{
		bin:    resolved,
		args:   append([]string{}, tokens[1:]...),
		name:   filepath.Base(bin),
		source: source,
	}, nil
}

// runDispatched executes a script under a non-JavaScript interpreter.
// The process machinery mirrors Run's: own process group, structured
// stderr streaming when a log writer is set, SIGTERM-then-SIGKILL on
// timeout, exit codes reported rather than errored.
func runDispatched(ctx context.Context, opts Options, d *dispatch) (*Result, error) {
	scriptPath := opts.File
	workDir := opts.WorkDir
	cleanup := func() {}
	if scriptPath != "" {
		abs, err := filepath.Abs(scriptPath)
		if err != nil {
			return nil, err
		}
		scriptPath = abs
	} else {
		// A snippet runs from a file of its own: no Prelude (that is
		// TypeScript for the generated client) and no .ts suffix (the
		// interpreter, not the name, decides what it is).
		if workDir == "" {
			wd, err := os.MkdirTemp("", "mcpx-run-")
			if err != nil {
				return nil, err
			}
			workDir = wd
			cleanup = func() { _ = os.RemoveAll(wd) }
		} else if err := os.MkdirAll(workDir, 0o700); err != nil {
			return nil, err
		}
		sum := sha256sum([]byte(opts.Source))
		scriptPath = filepath.Join(workDir, "script-"+sum[:16])
		if err := writeIfChanged(scriptPath, opts.Source); err != nil {
			cleanup()
			return nil, err
		}
	}
	defer cleanup()
	if workDir == "" {
		workDir = filepath.Dir(scriptPath)
	}

	dir, err := ResolveCwd(opts.CwdPolicy, opts.Dir, scriptPath, opts.ConfigDir, workDir)
	if err != nil {
		return nil, err
	}

	notice := fmt.Sprintf("mcpx: running under %s (%s dispatch): no generated mcpx client is "+
		"available to a non-JavaScript script, and script.permissions profiles are not enforced "+
		"for this interpreter; MCPX_ENDPOINT, MCPX_SESSION and MCPX_SOCKET are in the environment",
		d.name, d.source)

	runCtx := ctx
	var cancel context.CancelFunc
	if opts.Timeout > 0 {
		runCtx, cancel = context.WithTimeout(ctx, opts.Timeout)
		defer cancel()
	}

	args := append(append([]string{}, d.args...), scriptPath)
	args = append(args, opts.Args...)
	cmd := exec.Command(d.bin, args...)
	cmd.Dir = dir
	cmd.Env = append(os.Environ(), "MCPX_RUNTIME="+d.name, "MCPX_ENTRY="+scriptPath, "MCPX_DISPATCH="+d.source)
	for k, v := range opts.Env {
		cmd.Env = append(cmd.Env, k+"="+v)
	}
	if opts.Stdout != nil {
		cmd.Stdout = opts.Stdout
	} else {
		cmd.Stdout = os.Stdout
	}
	var stderrDone chan struct{}
	if opts.Log != nil {
		pr, pw, perr := os.Pipe()
		if perr != nil {
			return nil, perr
		}
		cmd.Stderr = pw
		passthrough := opts.Stderr
		if passthrough == nil {
			passthrough = os.Stderr
		}
		stderrDone = make(chan struct{})
		go func() {
			defer close(stderrDone)
			defer pr.Close()
			_ = logging.Stream(pr, opts.Log, logging.StreamOptions{
				Enrich:      opts.Enrich,
				Passthrough: passthrough,
				Collect:     opts.CollectLogs,
				Result:      opts.OnResult,
				Artifact:    opts.OnArtifact,
			})
		}()
		// The notice rides the same stream as the script's own stderr,
		// ahead of anything the script prints.
		fmt.Fprintln(pw, notice)
		defer func() {
			pw.Close()
			<-stderrDone
		}()
	} else if opts.Stderr != nil {
		cmd.Stderr = opts.Stderr
		fmt.Fprintln(opts.Stderr, notice)
	} else {
		cmd.Stderr = os.Stderr
		fmt.Fprintln(os.Stderr, notice)
	}
	if opts.Stdin != nil {
		cmd.Stdin = opts.Stdin
	} else {
		cmd.Stdin = os.Stdin
	}
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}

	start := time.Now()
	if err := cmd.Start(); err != nil {
		return nil, fmt.Errorf("start %s: %w", d.name, err)
	}
	waitCh := make(chan error, 1)
	go func() { waitCh <- cmd.Wait() }()

	res := &Result{Runtime: d.name, Script: scriptPath, Notice: notice}
	select {
	case <-runCtx.Done():
		res.TimedOut = true
		if cmd.Process != nil {
			_ = syscall.Kill(-cmd.Process.Pid, syscall.SIGTERM)
			select {
			case <-waitCh:
			case <-time.After(defaults.ShutdownGrace):
				_ = syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL)
				<-waitCh
			}
		}
		res.Duration = time.Since(start)
		res.ExitCode = 124
		return res, fmt.Errorf("script timed out after %s", opts.Timeout)
	case err := <-waitCh:
		res.Duration = time.Since(start)
		if err != nil {
			var ee *exec.ExitError
			if errors.As(err, &ee) {
				res.ExitCode = ee.ExitCode()
				return res, nil
			}
			return res, err
		}
		return res, nil
	}
}
