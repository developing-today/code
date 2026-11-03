package daemon

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os/exec"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
	"github.com/developing-today/code/pkgs/mcpx/internal/schemaval"
)

// scriptWaitDelay bounds how long a script view's call waits on its output
// pipes after the program exits, so a grandchild that inherited them cannot
// hold the call open.
const scriptWaitDelay = 2 * time.Second

// scriptViewTool is the tool a script-backed view advertises (#1933 slice 2):
// its own name, its declared description, and its declared inputSchema, or an
// object with no properties when it declares none.
func scriptViewTool(name string, tv config.ToolView) mcpclient.Tool {
	schema := tv.InputSchema
	if len(schema) == 0 {
		schema = json.RawMessage(`{"type":"object"}`)
	}
	return mcpclient.Tool{Name: name, Description: tv.Description, InputSchema: schema}
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
func callScriptView(ctx context.Context, tool string, tv *config.ToolView, args any) (json.RawMessage, error) {
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

	timeout := config.DefaultCallTimeout
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
