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
func (r *Registry) runScriptView(ctx context.Context, pl *pool.Pool, tool string, tv *config.ToolView, args any, session string) (json.RawMessage, error) {
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
	return callScriptView(ctx, pl, tool, tv, args)
}

func callScriptView(ctx context.Context, pl *pool.Pool, tool string, tv *config.ToolView, args any) (json.RawMessage, error) {
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
	mcpclient.NoteToolCallDispatched(ctx)
	res, runErr := s.ExecService().Run(ctx, execsvc.Request{Source: rendered, SourceName: view.Path, Opts: opts}, nil)
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
	res, err := r.runScriptView(callCtx, pl, tool, tv, args, session)
	var invalid *mcpclient.InvalidParamsError
	if err != nil && dispatched.Load() && !tv.ReadOnly && r.writeLedger != nil &&
		!mcpclient.IsConclusiveUpstreamError(err) && !errors.As(err, &invalid) {
		return nil, r.recordUnknownOutcome(pl, server, tool, args, surface)
	}
	return res, err
}
