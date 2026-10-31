package mcpclient

import (
	"context"
	"encoding/json"
	"fmt"
	"time"
)

// Client-side task execution for tools whose execution.taskSupport is
// "required" (#1556, Phase 2 of #1554).
//
// A pooled tool that declares taskSupport "required" cannot be called the
// plain way: its server demands the call run as a task. CallTool resolves
// the named tool's declaration itself -- from the per-Client cached
// tools/list snapshot every ListTools refreshes -- and, for a "required" tool,
// drives the era's task flow to the terminal result, so both CallTool
// callers (the pool and the in-client wrapper) share the behaviour with
// no signature change. Every other declaration (absent, "optional",
// "forbidden", or a tool the snapshot does not know) follows the plain
// call path exactly as before.
//
// The eras differ, as they do server-side (internal/mcpserver/tasks.go):
//
//   - Legacy (2025-11-25 core tasks): the call is client-directed. The
//     request carries a task field, the answer is {task} at once, the
//     client polls tasks/get at the task's poll interval, and the
//     terminal payload comes from tasks/result.
//   - Modern (2026-07-28 tasks extension): the call is server-directed.
//     The client declares extensions["io.modelcontextprotocol/tasks"]
//     on every request (see capabilities), sends the plain call, and a
//     flat resultType "task" answer is polled with tasks/get, whose
//     DetailedTask inlines the terminal result or error -- there is no
//     tasks/result in the extension.
//
// Bounds: the task's TTL is the outer bound on polling, and the caller's
// context bounds it first -- a cancel or deadline sends tasks/cancel
// best-effort and returns the context error. A malformed task response
// (no task handle, wrong shape, unknown status) is a call error, never
// a hang and never an unbounded poll.

// taskSupportRequired is the execution.taskSupport value that demands
// task-augmented execution.
const taskSupportRequired = "required"

// Task statuses, from the specification (mirrors internal/tasks).
const (
	taskStatusWorking       = "working"
	taskStatusInputRequired = "input_required"
	taskStatusCompleted     = "completed"
	taskStatusFailed        = "failed"
	taskStatusCancelled     = "cancelled"
)

// TaskInputRequiredError is the distinct named error for a task that
// stopped in input_required: the upstream task asked a question, and
// this phase of mcpx does not relay an upstream task's inputRequests
// downstream (an open question on #1554, explicitly out of scope for
// #1556). Callers can errors.As for it and decide what a question they
// cannot answer means for them.
type TaskInputRequiredError struct {
	Tool   string
	TaskID string
}

func (e *TaskInputRequiredError) Error() string {
	return fmt.Sprintf("mcpx: tool %s: upstream task %s is input_required: the task asked a question this phase of mcpx does not relay", e.Tool, e.TaskID)
}

// parseTaskSupport reads execution.taskSupport out of a tool's raw
// Execution. Anything unparseable or absent is "", which the caller
// treats as "no requirement" -- the plain path.
func parseTaskSupport(raw json.RawMessage) string {
	var ex struct {
		TaskSupport string `json:"taskSupport"`
	}
	_ = json.Unmarshal(raw, &ex)
	return ex.TaskSupport
}

// resolveTaskSupport is the named tool's taskSupport from the cached
// snapshot -- and only from the snapshot. CallTool must not grow a
// tools/list round trip of its own: a server may never answer one, and
// a call would hang on a question its caller never asked (the same
// rule the re-issue gate follows; see toolCallReissueAllowed). The
// snapshot is populated by every ListTools this client serves, and the
// pool lists the catalogue on its pooled clients as its normal
// operation (refreshSchemas), so a client serving pooled calls has
// listed the server it calls. A client that has never listed resolves
// "" -- the plain path, exactly what it did before task execution
// existed.
func (c *Client) resolveTaskSupport(name string) string {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.toolExec[name]
}

// taskPollInterval is how long to wait between tasks/get polls: the
// interval the task itself declared (milliseconds), or one second when
// it declared none.
func taskPollInterval(ms int64) time.Duration {
	if ms <= 0 {
		return time.Second
	}
	return time.Duration(ms) * time.Millisecond
}

// cancelTask sends tasks/cancel best-effort on a context that survives
// the caller's cancellation, bounded so a dead server cannot hang the
// return. The outcome is deliberately ignored: the caller is already
// getting the context error that caused the cancel.
func (c *Client) cancelTask(taskID string) {
	ctx, cancel := context.WithTimeout(context.WithoutCancel(context.Background()), 5*time.Second)
	defer cancel()
	params, _ := json.Marshal(map[string]string{"taskId": taskID})
	var raw json.RawMessage
	_ = c.call(ctx, "tasks/cancel", params, &raw)
}

// waitTaskPoll sleeps one poll interval, or ends the wait: the caller's
// context finishing cancels the task and returns its error, and the
// task's TTL expiring cancels it and returns a timeout error.
func (c *Client) waitTaskPoll(ctx context.Context, tool, taskID string, interval time.Duration, deadline time.Time) error {
	t := time.NewTimer(interval)
	defer t.Stop()
	select {
	case <-ctx.Done():
		c.cancelTask(taskID)
		return ctx.Err()
	case <-t.C:
	}
	if !deadline.IsZero() && !time.Now().Before(deadline) {
		c.cancelTask(taskID)
		return fmt.Errorf("mcpx: tool %s: task %s did not finish within its TTL", tool, taskID)
	}
	return nil
}

func taskDeadline(start time.Time, ttlMs int64) time.Time {
	if ttlMs <= 0 {
		return time.Time{}
	}
	return start.Add(time.Duration(ttlMs) * time.Millisecond)
}

// legacyTask is the 2025-11-25 Task shape tasks/get answers and the
// {task} handle a task-augmented tools/call returns.
type legacyTask struct {
	TaskID        string `json:"taskId"`
	Status        string `json:"status"`
	StatusMessage string `json:"statusMessage,omitempty"`
	TTL           int64  `json:"ttl"`
	PollInterval  int64  `json:"pollInterval"`
}

// callToolAsLegacyTask runs a taskSupport-"required" call the legacy
// way: task-augmented tools/call, tasks/get polls, tasks/result for the
// terminal payload. Faults surface through tasks/result, whose error is
// the server's own rpcError -- the client's existing error shape.
func (c *Client) callToolAsLegacyTask(ctx context.Context, name string, args any) (json.RawMessage, error) {
	params, err := json.Marshal(map[string]any{
		"name": name, "arguments": args, "task": map[string]any{},
	})
	if err != nil {
		return nil, err
	}
	var created json.RawMessage
	if err := c.call(ctx, "tools/call", params, &created); err != nil {
		return nil, err
	}
	var wrap struct {
		Task *legacyTask `json:"task"`
	}
	if err := json.Unmarshal(created, &wrap); err != nil || wrap.Task == nil || wrap.Task.TaskID == "" {
		return nil, fmt.Errorf("mcpx: tool %s: malformed task-augmented response: expected a {task} handle, got %s", name, shortRaw(created))
	}
	task := wrap.Task
	start := time.Now()
	deadline := taskDeadline(start, task.TTL)
	getParams, _ := json.Marshal(map[string]string{"taskId": task.TaskID})
	for {
		switch task.Status {
		case taskStatusCompleted, taskStatusFailed:
			var raw json.RawMessage
			if err := c.call(ctx, "tasks/result", getParams, &raw); err != nil {
				return nil, err
			}
			if task.Status == taskStatusCompleted {
				c.checkToolOutput(ctx, name, raw)
			}
			return raw, nil
		case taskStatusCancelled:
			return nil, fmt.Errorf("mcpx: tool %s: task %s was cancelled", name, task.TaskID)
		case taskStatusInputRequired:
			return nil, &TaskInputRequiredError{Tool: name, TaskID: task.TaskID}
		case taskStatusWorking:
		case "":
			return nil, fmt.Errorf("mcpx: tool %s: malformed task response: task %s carries no status", name, task.TaskID)
		default:
			return nil, fmt.Errorf("mcpx: tool %s: malformed task response: unknown task status %q", name, task.Status)
		}
		if err := c.waitTaskPoll(ctx, name, task.TaskID, taskPollInterval(task.PollInterval), deadline); err != nil {
			return nil, err
		}
		var snap legacyTask
		if err := c.call(ctx, "tasks/get", getParams, &snap); err != nil {
			if ctx.Err() != nil {
				c.cancelTask(task.TaskID)
				return nil, ctx.Err()
			}
			return nil, err
		}
		if snap.TaskID == "" {
			return nil, fmt.Errorf("mcpx: tool %s: malformed tasks/get response: no taskId", name)
		}
		// The snapshot replaces the handle whole, except the poll
		// interval and TTL, which a server may restate or drop; keep
		// the last known values when it drops them.
		if snap.PollInterval <= 0 {
			snap.PollInterval = task.PollInterval
		}
		if snap.TTL <= 0 {
			snap.TTL = task.TTL
		}
		task = &snap
	}
}

// modernTaskResult is both modern shapes this client reads: the flat
// CreateTaskResult a tools/call answers (resultType "task") and the
// DetailedTask tasks/get answers, which inlines a terminal task's
// result or error.
type modernTaskResult struct {
	ResultType     string          `json:"resultType"`
	TaskID         string          `json:"taskId"`
	Status         string          `json:"status"`
	StatusMessage  string          `json:"statusMessage,omitempty"`
	TTLMs          int64           `json:"ttlMs"`
	PollIntervalMs int64           `json:"pollIntervalMs"`
	Result         json.RawMessage `json:"result,omitempty"`
	Error          *rpcError       `json:"error,omitempty"`
}

// isTaskResult reports whether a modern tools/call result is a task
// handle rather than the call's own result.
func isTaskResult(raw json.RawMessage) bool {
	var t modernTaskResult
	return json.Unmarshal(raw, &t) == nil && t.ResultType == "task"
}

// finishModernTask polls a modern task handle from CallTool to its
// terminal state and returns the inlined result. It runs only for
// taskSupport-"required" tools: a task answer for any other tool is the
// server's eager-window behaviour and is returned to the caller as-is.
func (c *Client) finishModernTask(ctx context.Context, name string, raw json.RawMessage) (json.RawMessage, error) {
	var task modernTaskResult
	if err := json.Unmarshal(raw, &task); err != nil || task.TaskID == "" {
		return nil, fmt.Errorf("mcpx: tool %s: malformed task response: expected a task handle with a taskId, got %s", name, shortRaw(raw))
	}
	start := time.Now()
	deadline := taskDeadline(start, task.TTLMs)
	getParams, _ := json.Marshal(map[string]string{"taskId": task.TaskID})
	fetched := false // a terminal status read from tasks/get, not the handle
	for {
		switch task.Status {
		case taskStatusCompleted:
			if len(task.Result) > 0 {
				return task.Result, nil
			}
			if fetched {
				return nil, fmt.Errorf("mcpx: tool %s: malformed tasks/get response: task %s completed with no inlined result", name, task.TaskID)
			}
		case taskStatusFailed:
			if task.Error != nil {
				return nil, task.Error
			}
			if fetched {
				return nil, fmt.Errorf("mcpx: tool %s: task %s failed: %s", name, task.TaskID, task.StatusMessage)
			}
		case taskStatusCancelled:
			return nil, fmt.Errorf("mcpx: tool %s: task %s was cancelled", name, task.TaskID)
		case taskStatusInputRequired:
			return nil, &TaskInputRequiredError{Tool: name, TaskID: task.TaskID}
		case taskStatusWorking:
		case "":
			return nil, fmt.Errorf("mcpx: tool %s: malformed task response: task %s carries no status", name, task.TaskID)
		default:
			return nil, fmt.Errorf("mcpx: tool %s: malformed task response: unknown task status %q", name, task.Status)
		}
		if err := c.waitTaskPoll(ctx, name, task.TaskID, taskPollInterval(task.PollIntervalMs), deadline); err != nil {
			return nil, err
		}
		var snap modernTaskResult
		if err := c.call(ctx, "tasks/get", getParams, &snap); err != nil {
			if ctx.Err() != nil {
				c.cancelTask(task.TaskID)
				return nil, ctx.Err()
			}
			return nil, err
		}
		if snap.TaskID == "" {
			return nil, fmt.Errorf("mcpx: tool %s: malformed tasks/get response: no taskId", name)
		}
		if snap.PollIntervalMs <= 0 {
			snap.PollIntervalMs = task.PollIntervalMs
		}
		if snap.TTLMs <= 0 {
			snap.TTLMs = task.TTLMs
		}
		task = snap
		fetched = true
	}
}

// shortRaw bounds a server payload quoted into an error message.
func shortRaw(raw json.RawMessage) string {
	const max = 120
	s := string(raw)
	if len(s) > max {
		return s[:max] + "..."
	}
	return s
}
