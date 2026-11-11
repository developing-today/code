package daemon

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync/atomic"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
	"github.com/developing-today/code/pkgs/mcpx/internal/execsvc"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
	"github.com/developing-today/code/pkgs/mcpx/internal/recipes"
	"github.com/developing-today/code/pkgs/mcpx/internal/schemaval"
)

// scriptWaitDelay bounds how long a script view's call waits on its output
// pipes after the program exits, so a grandchild that inherited them cannot
// hold the call open.
const scriptWaitDelay = 2 * time.Second

// ScriptAnnotations is the behaviour hints a script-backed view declares
// (#1933 slice 2). A script has no upstream annotations, so the declaration
// is the only classification input the confirmation and write policy have:
// readOnlyHint is the declaration's own, and destructiveHint defaults to true
// when the script does not say otherwise -- the specification's conservative
// default, and the one that keeps an unclassified script behind the floor.
func ScriptAnnotations(tv config.ToolView) json.RawMessage {
	destructive := true
	if tv.Destructive != nil {
		destructive = *tv.Destructive
	}
	if tv.ReadOnly {
		destructive = false
	}
	b, _ := json.Marshal(map[string]bool{
		"readOnlyHint":    tv.ReadOnly,
		"destructiveHint": destructive,
	})
	return b
}

// scriptViewTool is the tool a script-backed view advertises (#1933 slice 2):
// its own name, its declared description, its declared (or generated)
// inputSchema, and the behaviour hints its declaration carries.
func scriptViewTool(name string, tv config.ToolView) mcpclient.Tool {
	schema := tv.InputSchema
	if len(schema) == 0 {
		schema = json.RawMessage(`{"type":"object"}`)
	}
	return mcpclient.Tool{
		Name:        name,
		Description: tv.Description,
		InputSchema: schema,
		Annotations: ScriptAnnotations(tv),
	}
}

// callScriptView runs one script-backed view. args already carry the view's
// pins. They are checked against the view's inputSchema, then written to the
// program's stdin as one JSON object. The program is executed directly with its
// declared arguments and never through a shell, so no caller value can become
// part of a command line.
//
// The result is an MCP CallToolResult, as an upstream tool returns one: stdout
// as text. A program that exits non-zero is a result with isError set, carrying
// its exit status and stderr. Only a call that cannot run, times out or is
// cancelled is an error.
func (r *Registry) runScriptView(ctx context.Context, pl *pool.Pool, tool string, tv *config.ToolView, args any, session string, resolved *config.Resolved) (json.RawMessage, error) {
	if tv.File != "" {
		r.mu.RLock()
		run := r.scriptRun
		r.mu.RUnlock()
		if run == nil {
			return nil, fmt.Errorf("tool %q: this daemon has no script runner configured", tool)
		}
		values := map[string]any{}
		switch a := args.(type) {
		case nil:
		case map[string]any:
			values = a
		default:
			b, err := json.Marshal(a)
			if err != nil {
				return nil, fmt.Errorf("tool %q: arguments could not be encoded: %w", tool, err)
			}
			if err := json.Unmarshal(b, &values); err != nil {
				return nil, &mcpclient.InvalidParamsError{Tool: tool, Err: fmt.Errorf("arguments must be an object: %w", err)}
			}
		}
		if len(tv.InputSchema) > 0 {
			compiled, err := schemaval.Compile(tv.InputSchema)
			if err != nil {
				return nil, fmt.Errorf("tool %q: inputSchema does not compile: %w", tool, err)
			}
			payload, _ := json.Marshal(values)
			if err := compiled.ValidateJSON(payload); err != nil {
				return nil, &mcpclient.InvalidParamsError{Tool: tool, Err: err}
			}
		}
		return run(ctx, tool, *tv, values, session)
	}
	return callScriptView(ctx, pl, tool, tv, args, resolved)
}

func callScriptView(ctx context.Context, pl *pool.Pool, tool string, tv *config.ToolView, args any, resolved *config.Resolved) (json.RawMessage, error) {
	ctx = withRequestedCallTimeout(ctx)
	payload, err := json.Marshal(args)
	if err != nil {
		return nil, fmt.Errorf("tool %q: arguments could not be encoded: %w", tool, err)
	}
	if string(payload) == "null" {
		payload = []byte("{}")
	}
	if len(tv.InputSchema) > 0 {
		compiled, err := schemaval.Compile(tv.InputSchema)
		if err != nil {
			return nil, fmt.Errorf("tool %q: inputSchema does not compile: %w", tool, err)
		}
		if err := compiled.ValidateJSON(json.RawMessage(payload)); err != nil {
			return nil, &mcpclient.InvalidParamsError{Tool: tool, Err: err}
		}
	}

	// An undeclared timeout inherits the pool's resolved call timeout
	// -- the same budget upstream calls get -- not the package default:
	// the declaration documents empty as "the pool's call timeout", and
	// falling back to DefaultCallTimeout ignored daemon/server budgets
	// in both directions (Codex P2 on PR #8317). The per-call request
	// lowering below still applies on top.
	timeout := config.DefaultCallTimeout
	if pl != nil {
		if resolved := pl.CallTimeout(ctx); resolved > 0 {
			timeout = resolved
		}
	}
	if d, err := time.ParseDuration(tv.Timeout); err == nil && d > 0 {
		timeout = d
	}
	if req, ok := pool.RequestedCallTimeout(ctx); ok && req < timeout {
		timeout = req
	}
	cctx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()

	cmd := exec.CommandContext(cctx, tv.Command, tv.Args...)
	if resolved != nil {
		cmd.Env = scriptCommandEnv(resolved)
	}
	cmd.Stdin = bytes.NewReader(payload)
	stdout := &cappedBuffer{max: defaults.ExecStdoutLimit}
	stderr := &cappedBuffer{max: defaults.ExecStderrLimit}
	cmd.Stdout, cmd.Stderr = stdout, stderr
	cmd.WaitDelay = scriptWaitDelay
	mcpclient.NoteToolCallDispatched(ctx)
	runErr := cmd.Run()
	if errors.Is(runErr, exec.ErrWaitDelay) {
		// The program exited and only its inherited pipes were still open.
		runErr = nil
	}

	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if cctx.Err() != nil {
		return nil, fmt.Errorf("tool %q did not finish within %s", tool, timeout)
	}
	var exitErr *exec.ExitError
	if runErr != nil && !errors.As(runErr, &exitErr) {
		return nil, fmt.Errorf("tool %q could not run %q: %w", tool, tv.Command, runErr)
	}

	content := []map[string]string{{"type": "text", "text": stdout.String()}}
	out := map[string]any{}
	if exitErr != nil {
		out["isError"] = true
		content = append(content, map[string]string{
			"type": "text",
			"text": fmt.Sprintf("exit status %d: %s", exitErr.ExitCode(), stderr.String()),
		})
	}
	out["content"] = content
	b, err := json.Marshal(out)
	if err != nil {
		return nil, err
	}
	return b, nil
}

// runDeclaredScript executes one file-backed tool view (#1933 slice 2).
//
// The declaration fixes every execution option: the script's path (and so
// its runtime, from its shebang or extension), its working directory, and
// its timeout when it declares one. The caller supplies the @param values
// and nothing else -- there is no ExecOptions on this path, so a tool's
// behaviour cannot change with who called it.
//
// The body is rendered from the script's own source with the values as JSON
// literals, exactly as a recipe is rendered, then run through the daemon's
// exec service against this daemon.
func (s *Server) runDeclaredScript(ctx context.Context, tool string, view config.ToolView, values map[string]any, session string) (json.RawMessage, error) {
	if view.Path == "" {
		return nil, fmt.Errorf("tool %q: script has no resolved path", tool)
	}
	src, err := os.ReadFile(view.Path)
	if err != nil {
		return nil, fmt.Errorf("tool %q: cannot read %s: %w", tool, view.Path, err)
	}
	rec := recipes.Recipe{
		Name:         tool,
		Path:         view.Path,
		Source:       string(src),
		Placeholders: view.Placeholders,
	}
	rendered, err := rec.Render(values)
	if err != nil {
		return nil, &mcpclient.InvalidParamsError{Tool: tool, Err: err}
	}
	// Preflight before anything runs: the exec endpoint refuses a
	// script that reaches a namespace whose required secret has no
	// value before its first statement, but a declared script
	// skipped that check, so its local side effects executed before
	// the eventual missing-secret refusal surfaced mid-run (Codex
	// P1 on PR #8317).
	if refusal := s.reg.refuseScript(rendered); refusal != nil {
		return nil, refusal
	}
	opts := execsvc.Options{
		Output:  execsvc.OutputStructured,
		Session: session,
		// The session is the caller's, borrowed for this one tool
		// call: the run must not release the caller's session-scoped
		// instances when it ends (Codex P1 on PR #8317).
		BorrowSession: session != "",
		// Relative imports and data files beside the script work from
		// the directory somebody put it in, not from the daemon's cwd.
		Cwd: filepath.Dir(view.Path),
	}
	if view.Timeout != "" {
		opts.Timeout = view.Timeout
	}
	// SourceName carries the declared script name as language evidence:
	// the runner executes Source (the rendered body), but ScriptLanguage
	// reads the shebang from the source and otherwise the file
	// extension, so without it a .py declaration with no shebang was
	// misrouted to the TypeScript runtime (Codex P2 on PR #8317).
	// Passing it as File instead would execute the unrendered file.
	// The execution's run id is chosen here, and when this call
	// belongs to an ask call (its context carries the ask id) or to
	// a live run (a file-backed call nested inside one), the run is
	// registered against that owner in the ask table: the script's
	// upstream calls join through the run id, and an unregistered
	// run's questions found no owner and never reached the caller's
	// ask poll (Codex P2 on PR #8317).
	runID := execsvc.NewRunID()
	if askID, ok := ctx.Value(askCallKey{}).(string); ok && askID != "" {
		s.reg.asks.attachRun(askID, runID)
	} else if parent := runFrom(ctx); parent != "" {
		s.reg.asks.attachRunToRun(parent, runID)
	}
	mcpclient.NoteToolCallDispatched(ctx)
	req := execsvc.Request{Source: rendered, SourceName: view.Path, Opts: opts, RunID: runID}
	cleanup := func() {}
	if file, cf, ferr := materializeDeclaredScript(view.Path, rendered); ferr == nil {
		// Run the rendered body as a file in a directory holding
		// the declared script's siblings, so a relative import
		// (./helper, a sibling module) resolves the way it does
		// for the declared file itself. As Source, the runner
		// materialised the body under a generated name in its
		// cache directory, where no sibling exists (Codex P2 on
		// PR #8317).
		req = execsvc.Request{File: file, Opts: opts, RunID: runID}
		cleanup = cf
	}
	defer cleanup()
	res, runErr := s.ExecService().Run(ctx, req, nil)
	if runErr != nil && res == nil {
		return nil, fmt.Errorf("tool %q: %w", tool, runErr)
	}
	if res == nil {
		return nil, fmt.Errorf("tool %q: the script produced no result", tool)
	}
	// The tool result carries what the script emitted as well as what
	// it printed: emit(...) values land in res.Emits, not Stdout, and
	// dropping them made an emit-only script return empty content
	// despite succeeding (Codex P2 on PR #8317).
	content := []map[string]string{{"type": "text", "text": res.Stdout}}
	for _, e := range res.Emits {
		content = append(content, map[string]string{"type": "text", "text": string(e)})
	}
	out := map[string]any{
		"content": content,
	}
	if res.ExitCode != 0 || res.Error != "" {
		out["isError"] = true
		detail := res.Error
		out["content"] = append(out["content"].([]map[string]string), map[string]string{
			"type": "text",
			"text": fmt.Sprintf("exit status %d: %s", res.ExitCode, detail),
		})
	}
	b, err := json.Marshal(out)
	if err != nil {
		return nil, err
	}
	return b, nil
}

// callScriptWithLedger runs a script-backed view under the same
// write-ledger discipline as an upstream call (Codex P1 on PR #8317):
// the script branches used to return runScriptView's error directly,
// so a non-read-only script that timed out or lost its caller left no
// unknown-outcome record and the ledger preflight could not block a
// retry -- the same side effect could run twice. Dispatch is noted by
// the execution paths themselves (NoteToolCallDispatched) at the
// moment the program starts; an error before that point, or a
// conclusive validation error, records nothing.
func (r *Registry) callScriptWithLedger(ctx context.Context, pl *pool.Pool, server, tool string, tv *config.ToolView, args any, session, surface string) (json.RawMessage, error) {
	var dispatched atomic.Bool
	callCtx := mcpclient.WithToolCallDispatchHook(ctx, func() { dispatched.Store(true) })
	r.mu.RLock()
	resolved := r.views[server]
	r.mu.RUnlock()
	res, err := r.runScriptView(callCtx, pl, tool, tv, args, session, resolved)
	var invalid *mcpclient.InvalidParamsError
	// Read-only is read-only however it was declared: the view's own
	// flag, or the server-level readOnlyTools list the policy guards
	// honor via declaredReadOnly (upstream callWithLedger exempts
	// both). Checking only tv.ReadOnly recorded an unknown write
	// outcome -- and a persistent write block -- for a tool the
	// operator had declared read-only at server level (Codex P2 on
	// PR #8317).
	if err != nil && dispatched.Load() && !tv.ReadOnly && !r.declaredReadOnly(server, tool) && r.writeLedger != nil &&
		!mcpclient.IsConclusiveUpstreamError(err) && !errors.As(err, &invalid) {
		return nil, r.recordUnknownOutcome(pl, server, tool, args, surface)
	}
	return res, err
}

// scriptCommandEnv is the environment a command-backed tool runs
// with: the daemon's own, overlaid with its server's configured
// env, overlaid with the resolved values of the secrets the server
// requires -- the values a child process of that server would see
// (config.Resolved.SecretValue, runtime provision first). Without
// it, a script-only namespace's env: entries and
// runtime-provisioned secrets never reached the program: cmd.Env
// was unset, so only the daemon's environment was inherited (Codex
// P2 on PR #8317).
func scriptCommandEnv(resolved *config.Resolved) []string {
	keys := []string{}
	vals := map[string]string{}
	set := func(k, v string) {
		if _, ok := vals[k]; !ok {
			keys = append(keys, k)
		}
		vals[k] = v
	}
	for _, kv := range os.Environ() {
		if k, v, ok := strings.Cut(kv, "="); ok {
			set(k, v)
		}
	}
	for k, v := range resolved.Env {
		set(k, v)
	}
	for _, name := range resolved.SecretNames() {
		if v, ok := resolved.SecretValue(name); ok {
			set(name, v)
		}
	}
	out := make([]string, 0, len(keys))
	for _, k := range keys {
		out = append(out, k+"="+vals[k])
	}
	return out
}

// materializeDeclaredScript writes the rendered body into a fresh
// directory under the declared script's own name, with every
// sibling of the declared script linked in beside it, and returns
// the path to run plus a cleanup for the directory. Relative
// imports resolve against the file's directory, so the rendered
// body has to sit where its siblings are visible; the declared
// directory itself is never written to.
func materializeDeclaredScript(path, rendered string) (string, func(), error) {
	dir := filepath.Dir(path)
	tmp, err := os.MkdirTemp("", "mcpx-declared-")
	if err != nil {
		return "", nil, err
	}
	cleanup := func() { _ = os.RemoveAll(tmp) }
	entries, err := os.ReadDir(dir)
	if err != nil {
		cleanup()
		return "", nil, err
	}
	for _, e := range entries {
		if e.Name() == filepath.Base(path) {
			continue
		}
		// Best effort: a sibling that cannot be linked (a socket,
		// a permission edge) must not stop the tool; the script's
		// own import of it fails with its own error, as it should.
		_ = os.Symlink(filepath.Join(dir, e.Name()), filepath.Join(tmp, e.Name()))
	}
	target := filepath.Join(tmp, filepath.Base(path))
	if err := os.WriteFile(target, []byte(rendered), 0o755); err != nil {
		cleanup()
		return "", nil, err
	}
	return target, cleanup, nil
}
